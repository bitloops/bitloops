use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskStatus, InitSessionRecord, SummaryBootstrapRunRecord,
    SummaryBootstrapStatus,
};
use crate::runtime_presentation::{lane_activity_label, task_kind_label};

use super::orchestration::{
    embeddings_bootstrap_outstanding_after_initial_sync, embeddings_follow_up_pending,
    summaries_follow_up_pending, summary_bootstrap_outstanding_after_initial_sync, task_failed,
};
use super::stats::{SessionWorkplaneStats, StatusCounts};
use super::tasks::{
    effective_task_id, follow_up_sync_status, initial_sync_status, task_status_is_completed,
    task_status_is_failed, task_status_is_terminal,
};
use super::types::{
    InitRuntimeLaneProgressView, InitRuntimeLaneQueueView, InitRuntimeLaneView,
    InitRuntimeLaneWarningView,
};

pub(crate) struct SummaryEmbeddingsLaneContext<'a> {
    pub(crate) initial_sync: Option<&'a DevqlTaskRecord>,
    pub(crate) follow_up_sync: Option<&'a DevqlTaskRecord>,
    pub(crate) embeddings_task: Option<&'a DevqlTaskRecord>,
    pub(crate) summary_run: Option<&'a SummaryBootstrapRunRecord>,
    pub(crate) current_state: StatusCounts,
    pub(crate) progress: Option<InitRuntimeLaneProgressView>,
    pub(crate) summaries_progress: Option<InitRuntimeLaneProgressView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncDependencyState {
    Ready,
    Failed,
    WaitingForInitialSync,
    WaitingForCurrentStateConsumer,
}

pub(crate) fn derive_sync_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    current_state: StatusCounts,
) -> InitRuntimeLaneView {
    if !session.selections.run_sync {
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
            effective_task_id(
                initial_sync,
                session.initial_sync_terminal.as_ref(),
                session.initial_sync_task_id.as_deref(),
            ),
            None,
            None,
            Vec::new(),
        );
    }
    if follow_up_sync_status(session, follow_up_sync).is_some_and(task_status_is_failed) {
        return failed_lane(
            Some("Running a follow-up sync failed".to_string()),
            current_state,
            effective_task_id(
                follow_up_sync,
                session.follow_up_sync_terminal.as_ref(),
                session.follow_up_sync_task_id.as_deref(),
            ),
            None,
            None,
            Vec::new(),
        );
    }
    if initial_sync_status(session, initial_sync).is_some_and(task_status_is_failed) {
        return failed_lane(
            Some("Syncing repository failed".to_string()),
            current_state,
            effective_task_id(
                initial_sync,
                session.initial_sync_terminal.as_ref(),
                session.initial_sync_task_id.as_deref(),
            ),
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

pub(crate) fn derive_ingest_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
) -> InitRuntimeLaneView {
    if !session.selections.run_ingest {
        return skipped_lane();
    }
    if let Some(task) = active_task(ingest_task) {
        return lane_from_task(
            task,
            Some("ingest".to_string()),
            StatusCounts::default(),
            None,
            Vec::new(),
        );
    }
    if let Some(task) = ingest_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Ingesting commit history failed".to_string()),
            StatusCounts::default(),
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if session.selections.run_sync
        && !initial_sync_status(session, initial_sync).is_some_and(task_status_is_terminal)
    {
        return runtime_lane("waiting", None, StatusCounts::default(), Vec::new())
            .with_waiting_reason("waiting_for_sync")
            .with_activity_label("Waiting for sync to complete before starting ingest");
    }
    if ingest_task.is_none() {
        return runtime_lane("waiting", None, StatusCounts::default(), Vec::new())
            .with_waiting_reason("queued")
            .with_activity_label("Waiting to start ingest");
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

pub(crate) fn derive_code_embeddings_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    current_state: StatusCounts,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if !session.selections.run_code_embeddings {
        return skipped_lane();
    }
    let warnings = stats.code_embedding_warnings();
    if let Some(task) = active_task(embeddings_task) {
        return lane_from_task(
            task,
            Some("embeddings_bootstrap".to_string()),
            stats.code_embedding_jobs.counts,
            progress,
            warnings,
        );
    }
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Preparing the embeddings runtime failed".to_string()),
            stats.code_embedding_jobs.counts,
            Some(task.task_id.clone()),
            None,
            progress,
            warnings,
        );
    }
    match sync_dependency_state(session, initial_sync, current_state) {
        SyncDependencyState::Failed => {
            return failed_lane(
                Some("Syncing repository failed".to_string()),
                stats.code_embedding_jobs.counts,
                effective_task_id(
                    initial_sync,
                    session.initial_sync_terminal.as_ref(),
                    session.initial_sync_task_id.as_deref(),
                ),
                None,
                progress,
                warnings,
            );
        }
        SyncDependencyState::WaitingForInitialSync => {
            return runtime_lane(
                "waiting",
                progress,
                stats.code_embedding_jobs.counts,
                warnings,
            )
            .with_waiting_reason("waiting_for_sync")
            .with_activity_label("Waiting for sync to complete before creating code embeddings");
        }
        SyncDependencyState::Ready | SyncDependencyState::WaitingForCurrentStateConsumer => {}
    }
    if embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task) {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_embeddings_bootstrap")
        .with_activity_label("Preparing the embeddings runtime");
    }
    if embeddings_follow_up_pending(session, initial_sync, follow_up_sync, embeddings_task) {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_follow_up_sync")
        .with_activity_label("Running a follow-up sync");
    }
    if sync_dependency_state(session, initial_sync, current_state)
        == SyncDependencyState::WaitingForCurrentStateConsumer
    {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_current_state_consumer")
        .with_activity_label("Applying codebase updates");
    }
    if let Some(reason) = stats.blocked_code_embedding_reason.clone() {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("blocked_mailbox")
        .with_activity_label("Indexing source code")
        .with_detail(reason);
    }
    if embedding_batches_are_preparing(stats, progress.as_ref()) {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("preparing_embedding_batches")
        .with_activity_label("Indexing first embedding batch");
    }
    if stats.code_embedding_jobs.counts.pending > 0 || stats.code_embedding_jobs.counts.running > 0
    {
        return runtime_lane(
            if stats.code_embedding_jobs.counts.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_activity_label("Indexing source code");
    }
    if !warnings.is_empty() {
        return runtime_lane(
            "warning",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_activity_label("Creating code embeddings");
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane(
            "waiting",
            progress,
            stats.code_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_workplane")
        .with_activity_label("Creating code embeddings");
    }
    completed_lane_with_progress(progress)
}

pub(crate) fn derive_summaries_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    current_state: StatusCounts,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if !session.selections.run_summaries {
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
    match sync_dependency_state(session, initial_sync, current_state) {
        SyncDependencyState::Failed => {
            return failed_lane(
                Some("Syncing repository failed".to_string()),
                stats.summary_jobs,
                effective_task_id(
                    initial_sync,
                    session.initial_sync_terminal.as_ref(),
                    session.initial_sync_task_id.as_deref(),
                ),
                summary_run
                    .map(|run| run.run_id.clone())
                    .or_else(|| {
                        session
                            .summary_bootstrap_terminal
                            .as_ref()
                            .map(|terminal| terminal.task_id.clone())
                    })
                    .or_else(|| session.summary_bootstrap_task_id.clone()),
                progress,
                warnings,
            );
        }
        SyncDependencyState::WaitingForInitialSync => {
            return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
                .with_waiting_reason("waiting_for_sync")
                .with_activity_label("Waiting for sync to complete before generating summaries")
                .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
        }
        SyncDependencyState::Ready | SyncDependencyState::WaitingForCurrentStateConsumer => {}
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
    if sync_dependency_state(session, initial_sync, current_state)
        == SyncDependencyState::WaitingForCurrentStateConsumer
    {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_current_state_consumer")
            .with_activity_label("Applying codebase updates")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
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

pub(crate) fn derive_summary_embeddings_lane(
    session: &InitSessionRecord,
    stats: &SessionWorkplaneStats,
    context: SummaryEmbeddingsLaneContext<'_>,
) -> InitRuntimeLaneView {
    let SummaryEmbeddingsLaneContext {
        initial_sync,
        follow_up_sync,
        embeddings_task,
        summary_run,
        current_state,
        progress,
        summaries_progress,
    } = context;

    if !session.selections.run_summary_embeddings {
        return skipped_lane();
    }
    let warnings = stats.summary_embedding_warnings();
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Preparing the embeddings runtime failed".to_string()),
            stats.summary_embedding_jobs.counts,
            Some(task.task_id.clone()),
            None,
            progress,
            warnings,
        );
    }
    if let Some(run) = summary_run
        && run.status == SummaryBootstrapStatus::Failed
    {
        return failed_lane(
            Some("Preparing summary generation failed".to_string()),
            stats.summary_embedding_jobs.counts,
            None,
            Some(run.run_id.clone()),
            progress,
            warnings,
        );
    }
    match sync_dependency_state(session, initial_sync, current_state) {
        SyncDependencyState::Failed => {
            return failed_lane(
                Some("Syncing repository failed".to_string()),
                stats.summary_embedding_jobs.counts,
                effective_task_id(
                    initial_sync,
                    session.initial_sync_terminal.as_ref(),
                    session.initial_sync_task_id.as_deref(),
                ),
                summary_run
                    .map(|run| run.run_id.clone())
                    .or_else(|| {
                        session
                            .summary_bootstrap_terminal
                            .as_ref()
                            .map(|terminal| terminal.task_id.clone())
                    })
                    .or_else(|| session.summary_bootstrap_task_id.clone()),
                progress,
                warnings,
            );
        }
        SyncDependencyState::WaitingForInitialSync => {
            return runtime_lane(
                "waiting",
                progress,
                stats.summary_embedding_jobs.counts,
                warnings,
            )
            .with_waiting_reason("waiting_for_sync")
            .with_activity_label(
                "Waiting for sync to complete before creating summary embeddings",
            );
        }
        SyncDependencyState::Ready | SyncDependencyState::WaitingForCurrentStateConsumer => {}
    }
    if embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task) {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_embeddings_bootstrap")
        .with_activity_label("Waiting for the embeddings runtime");
    }
    if summary_bootstrap_outstanding_after_initial_sync(session, initial_sync, summary_run) {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_summary_bootstrap")
        .with_activity_label("Waiting for summary generation to be ready");
    }
    if summaries_follow_up_pending(session, initial_sync, follow_up_sync, summary_run) {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_follow_up_sync")
        .with_activity_label("Running a follow-up sync");
    }
    if sync_dependency_state(session, initial_sync, current_state)
        == SyncDependencyState::WaitingForCurrentStateConsumer
    {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_current_state_consumer")
        .with_activity_label("Applying codebase updates");
    }
    if let Some(reason) = stats.blocked_summary_embedding_reason.clone() {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("blocked_mailbox")
        .with_activity_label("Creating summary embeddings")
        .with_detail(reason);
    }
    if stats.summary_embedding_jobs.counts.pending > 0
        || stats.summary_embedding_jobs.counts.running > 0
    {
        return runtime_lane(
            if stats.summary_embedding_jobs.counts.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_activity_label("Creating summary embeddings");
    }
    if stats.summary_jobs.pending > 0 || stats.summary_jobs.running > 0 {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_summaries")
        .with_activity_label("Waiting for summaries to be ready");
    }
    if progress_has_remaining(summaries_progress.as_ref()) {
        return runtime_lane(
            "warning",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_activity_label("Waiting for summaries to be ready")
        .with_detail(
            "Summary generation finished without producing current summaries for every eligible artefact"
                .to_string(),
        );
    }
    if !warnings.is_empty() {
        return runtime_lane(
            "warning",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_activity_label("Creating summary embeddings");
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane(
            "waiting",
            progress,
            stats.summary_embedding_jobs.counts,
            warnings,
        )
        .with_waiting_reason("waiting_for_workplane")
        .with_activity_label("Creating summary embeddings");
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

fn progress_visible_completed(progress: Option<&InitRuntimeLaneProgressView>) -> u64 {
    progress
        .map(|progress| {
            progress
                .completed
                .saturating_add(progress.in_memory_completed)
        })
        .unwrap_or(0)
}

fn embedding_batches_are_preparing(
    stats: &SessionWorkplaneStats,
    progress: Option<&InitRuntimeLaneProgressView>,
) -> bool {
    stats.code_embedding_jobs.counts.running > 0
        && stats.code_embedding_jobs.counts.completed == 0
        && progress_visible_completed(progress) == 0
}

fn sync_dependency_state(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    current_state: StatusCounts,
) -> SyncDependencyState {
    if !session.selections.run_sync {
        return SyncDependencyState::Ready;
    }

    if initial_sync_status(session, initial_sync).is_some_and(task_status_is_failed)
        || current_state.failed > 0
    {
        return SyncDependencyState::Failed;
    }
    if !initial_sync_status(session, initial_sync).is_some_and(task_status_is_completed) {
        return SyncDependencyState::WaitingForInitialSync;
    }
    if current_state.pending > 0 || current_state.running > 0 {
        return SyncDependencyState::WaitingForCurrentStateConsumer;
    }
    SyncDependencyState::Ready
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
