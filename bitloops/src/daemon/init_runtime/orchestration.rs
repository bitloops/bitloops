use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskStatus, InitSessionRecord, SummaryBootstrapRunRecord,
    SummaryBootstrapStatus,
};

use super::lanes::{active_task, running_task};
use super::stats::{SessionWorkplaneStats, StatusCounts, merge_status_counts};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SelectedSessionWorkplaneStats {
    pub(crate) embedding_jobs: StatusCounts,
    pub(crate) summary_jobs: StatusCounts,
    pub(crate) blocked_embedding_reason: Option<String>,
    pub(crate) blocked_summary_reason: Option<String>,
    pub(crate) warning_failed_jobs_total: u64,
}

pub(crate) fn record_task_completion_seq(session: &mut InitSessionRecord, task: &DevqlTaskRecord) {
    if task.status != DevqlTaskStatus::Completed {
        return;
    }
    if session.initial_sync_task_id.as_deref() == Some(task.task_id.as_str()) {
        assign_completion_seq(
            &mut session.next_completion_seq,
            &mut session.initial_sync_completion_seq,
        );
        return;
    }
    if session.follow_up_sync_task_id.as_deref() == Some(task.task_id.as_str()) {
        assign_completion_seq(
            &mut session.next_completion_seq,
            &mut session.follow_up_sync_completion_seq,
        );
        return;
    }
    if session.embeddings_bootstrap_task_id.as_deref() == Some(task.task_id.as_str()) {
        assign_completion_seq(
            &mut session.next_completion_seq,
            &mut session.embeddings_bootstrap_completion_seq,
        );
        return;
    }
    if session.summary_bootstrap_task_id.as_deref() == Some(task.task_id.as_str()) {
        assign_completion_seq(
            &mut session.next_completion_seq,
            &mut session.summary_bootstrap_completion_seq,
        );
    }
}

fn assign_completion_seq(next_completion_seq: &mut u64, target: &mut Option<u64>) {
    if target.is_some() {
        return;
    }
    *next_completion_seq += 1;
    *target = Some(*next_completion_seq);
}

fn latest_completed_sync_seq(session: &InitSessionRecord) -> Option<u64> {
    session
        .initial_sync_completion_seq
        .max(session.follow_up_sync_completion_seq)
}

pub(crate) fn session_requires_semantic_follow_up(session: &InitSessionRecord) -> bool {
    session.selections.embeddings_bootstrap.is_some()
        || session.selections.summaries_bootstrap.is_some()
}

pub(crate) fn selected_session_workplane_stats(
    session: &InitSessionRecord,
    stats: &SessionWorkplaneStats,
) -> SelectedSessionWorkplaneStats {
    let include_code_embeddings = session.selections.run_code_embeddings;
    let include_summary_embeddings = session.selections.run_summary_embeddings;
    let include_summaries = session.selections.run_summaries;

    let embedding_jobs = merge_status_counts([
        if include_code_embeddings {
            stats.code_embedding_jobs.counts
        } else {
            StatusCounts::default()
        },
        if include_summary_embeddings {
            stats.summary_embedding_jobs.counts
        } else {
            StatusCounts::default()
        },
    ]);
    let summary_jobs = if include_summaries {
        stats.summary_refresh_jobs.counts
    } else {
        StatusCounts::default()
    };

    let blocked_embedding_reason =
        if include_code_embeddings && stats.code_embedding_jobs.counts.has_pending_or_running() {
            stats.blocked_code_embedding_reason.clone()
        } else if include_summary_embeddings
            && stats.summary_embedding_jobs.counts.has_pending_or_running()
        {
            stats.blocked_summary_embedding_reason.clone()
        } else {
            None
        };
    let blocked_summary_reason =
        if include_summaries && stats.summary_refresh_jobs.counts.has_pending_or_running() {
            stats.blocked_summary_reason.clone()
        } else {
            None
        };

    SelectedSessionWorkplaneStats {
        embedding_jobs,
        summary_jobs,
        blocked_embedding_reason,
        blocked_summary_reason,
        warning_failed_jobs_total: u64::from(include_code_embeddings)
            * stats.code_embedding_jobs.counts.failed
            + u64::from(include_summary_embeddings) * stats.summary_embedding_jobs.counts.failed
            + u64::from(include_summaries) * stats.summary_refresh_jobs.counts.failed,
    }
}

pub(crate) fn task_failed(task: Option<&DevqlTaskRecord>) -> bool {
    task.is_some_and(|task| {
        matches!(
            task.status,
            DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
        )
    })
}

pub(crate) fn summary_run_failed(run: &SummaryBootstrapRunRecord) -> bool {
    run.status == SummaryBootstrapStatus::Failed
}

pub(crate) fn semantic_bootstraps_terminal(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    let embeddings_terminal = if session.selections.embeddings_bootstrap.is_some() {
        embeddings_task.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    let summaries_terminal = if session.selections.summaries_bootstrap.is_some() {
        summary_run.is_some_and(|run| {
            matches!(
                run.status,
                SummaryBootstrapStatus::Completed | SummaryBootstrapStatus::Failed
            )
        })
    } else {
        true
    };
    embeddings_terminal && summaries_terminal
}

pub(crate) fn semantic_bootstraps_ready(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    let embeddings_ready = if session.selections.embeddings_bootstrap.is_some() {
        embeddings_task.is_some_and(|task| task.status == DevqlTaskStatus::Completed)
    } else {
        true
    };
    let summaries_ready = if session.selections.summaries_bootstrap.is_some() {
        summary_run.is_some_and(|run| run.status == SummaryBootstrapStatus::Completed)
    } else {
        true
    };
    embeddings_ready && summaries_ready
}

pub(crate) fn semantic_bootstrap_waiting_reason(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> Option<&'static str> {
    let embeddings_waiting = session.selections.embeddings_bootstrap.is_some()
        && !embeddings_task.is_some_and(|task| task.status == DevqlTaskStatus::Completed);
    let summaries_waiting = session.selections.summaries_bootstrap.is_some()
        && !summary_run.is_some_and(|run| run.status == SummaryBootstrapStatus::Completed);

    match (embeddings_waiting, summaries_waiting) {
        (true, false) => Some("waiting_for_embeddings_bootstrap"),
        (false, true) => Some("waiting_for_summary_bootstrap"),
        (true, true) => Some("waiting_for_semantic_bootstrap"),
        (false, false) => None,
    }
}

pub(crate) fn embeddings_bootstrap_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    _initial_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
) -> bool {
    session.selections.embeddings_bootstrap.is_some()
        && session.initial_sync_completion_seq.is_some()
        && session.embeddings_bootstrap_completion_seq.is_none()
        && !task_failed(embeddings_task)
}

pub(crate) fn summary_bootstrap_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    _initial_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    session.selections.summaries_bootstrap.is_some()
        && session.initial_sync_completion_seq.is_some()
        && session.summary_bootstrap_completion_seq.is_none()
        && !summary_run.is_some_and(summary_run_failed)
}

pub(crate) fn embeddings_follow_up_pending(
    session: &InitSessionRecord,
    _initial_sync: Option<&DevqlTaskRecord>,
    _follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
) -> bool {
    if !session.follow_up_sync_required {
        return false;
    }
    if session.selections.embeddings_bootstrap.is_none() {
        return false;
    }
    if task_failed(embeddings_task) {
        return false;
    }
    let Some(bootstrap_completed_seq) = session.embeddings_bootstrap_completion_seq else {
        return false;
    };
    let Some(sync_completed_seq) = latest_completed_sync_seq(session) else {
        return false;
    };
    bootstrap_completed_seq > sync_completed_seq
}

pub(crate) fn summaries_follow_up_pending(
    session: &InitSessionRecord,
    _initial_sync: Option<&DevqlTaskRecord>,
    _follow_up_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    if !session.follow_up_sync_required {
        return false;
    }
    if session.selections.summaries_bootstrap.is_none() {
        return false;
    }
    if summary_run.is_some_and(summary_run_failed) {
        return false;
    }
    let Some(bootstrap_completed_seq) = session.summary_bootstrap_completion_seq else {
        return false;
    };
    let Some(sync_completed_seq) = latest_completed_sync_seq(session) else {
        return false;
    };
    bootstrap_completed_seq > sync_completed_seq
}

pub(crate) fn semantic_bootstrap_still_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task)
        || summary_bootstrap_outstanding_after_initial_sync(session, initial_sync, summary_run)
}

pub(crate) fn semantic_follow_up_ready_for_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    embeddings_follow_up_pending(session, initial_sync, follow_up_sync, embeddings_task)
        || summaries_follow_up_pending(session, initial_sync, follow_up_sync, summary_run)
}

pub(crate) fn semantic_follow_up_pending(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    running_task(follow_up_sync).is_some()
        || semantic_bootstrap_still_outstanding_after_initial_sync(
            session,
            initial_sync,
            embeddings_task,
            summary_run,
        )
        || semantic_follow_up_ready_for_sync(
            session,
            initial_sync,
            follow_up_sync,
            embeddings_task,
            summary_run,
        )
}

pub(crate) fn selected_top_level_terminal(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
) -> bool {
    let sync_terminal = if session.selections.run_sync {
        initial_sync.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    let ingest_terminal = if session.selections.run_ingest {
        ingest_task.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    sync_terminal && ingest_terminal
}

pub(crate) fn session_has_remaining_work(
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    current_state: StatusCounts,
    selected_workplane: &SelectedSessionWorkplaneStats,
) -> bool {
    active_task(initial_sync).is_some()
        || active_task(ingest_task).is_some()
        || active_task(follow_up_sync).is_some()
        || active_task(embeddings_task).is_some()
        || summary_run.is_some_and(|run| {
            matches!(
                run.status,
                SummaryBootstrapStatus::Queued | SummaryBootstrapStatus::Running
            )
        })
        || current_state.has_pending_or_running()
        || selected_workplane.embedding_jobs.has_pending_or_running()
        || selected_workplane.summary_jobs.has_pending_or_running()
}

pub(crate) fn session_fatal_failure_detail(
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    stats: &SessionWorkplaneStats,
) -> Option<String> {
    if let Some(task) = initial_sync
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Syncing repository", task));
    }
    if let Some(task) = ingest_task
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Ingesting commit history", task));
    }
    if let Some(task) = follow_up_sync
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Running a follow-up sync", task));
    }
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return Some(task_failure_detail(
            "Preparing the embeddings runtime",
            task,
        ));
    }
    if let Some(run) = summary_run
        && summary_run_failed(run)
    {
        return Some(summary_bootstrap_failure_detail(run));
    }
    if let Some(detail) = stats.failed_current_state_detail.clone() {
        return Some(detail);
    }
    None
}

fn task_failure_detail(label: &str, task: &DevqlTaskRecord) -> String {
    let error = task
        .error
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("task ended with status {}", task.status));
    format!("{label} failed: {error}")
}

fn summary_bootstrap_failure_detail(run: &SummaryBootstrapRunRecord) -> String {
    format!(
        "Preparing summary generation failed{}",
        run.error
            .as_deref()
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    )
}
