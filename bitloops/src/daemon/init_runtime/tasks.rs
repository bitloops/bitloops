use anyhow::Result;

use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskStatus, InitSessionRecord, InitSessionTaskTerminalSnapshot,
    SummaryBootstrapRunRecord, SummaryBootstrapStatus,
};

pub(crate) fn load_task_by_id(task_id: Option<&str>) -> Result<Option<DevqlTaskRecord>> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };
    crate::daemon::shared_devql_task_coordinator().task(task_id)
}

pub(crate) fn load_summary_task_by_id(task_id: Option<&str>) -> Result<Option<DevqlTaskRecord>> {
    Ok(load_task_by_id(task_id)?
        .filter(|task| task.kind == crate::daemon::DevqlTaskKind::SummaryBootstrap))
}

pub(crate) fn summary_run_from_task(task: DevqlTaskRecord) -> Option<SummaryBootstrapRunRecord> {
    let request = task.summary_bootstrap_spec()?.clone();
    let init_session_id = task.init_session_id.clone()?;
    let status = summary_status_from_task_status(task.status);
    let progress = task
        .summary_bootstrap_progress()
        .cloned()
        .unwrap_or_default();
    let result = task.summary_bootstrap_result().cloned();
    Some(SummaryBootstrapRunRecord {
        run_id: task.task_id,
        repo_id: task.repo_id,
        repo_root: task.repo_root,
        init_session_id,
        request,
        status,
        progress,
        result,
        error: task.error,
        submitted_at_unix: task.submitted_at_unix,
        started_at_unix: task.started_at_unix,
        updated_at_unix: task.updated_at_unix,
        completed_at_unix: task.completed_at_unix,
    })
}

pub(crate) fn summary_run_from_task_ref(
    task: &DevqlTaskRecord,
) -> Option<SummaryBootstrapRunRecord> {
    let request = task.summary_bootstrap_spec()?.clone();
    Some(SummaryBootstrapRunRecord {
        run_id: task.task_id.clone(),
        repo_id: task.repo_id.clone(),
        repo_root: task.repo_root.clone(),
        init_session_id: task.init_session_id.clone()?,
        request,
        status: summary_status_from_task_status(task.status),
        progress: task
            .summary_bootstrap_progress()
            .cloned()
            .unwrap_or_default(),
        result: task.summary_bootstrap_result().cloned(),
        error: task.error.clone(),
        submitted_at_unix: task.submitted_at_unix,
        started_at_unix: task.started_at_unix,
        updated_at_unix: task.updated_at_unix,
        completed_at_unix: task.completed_at_unix,
    })
}

pub(crate) fn summary_status_from_task_status(status: DevqlTaskStatus) -> SummaryBootstrapStatus {
    match status {
        DevqlTaskStatus::Queued => SummaryBootstrapStatus::Queued,
        DevqlTaskStatus::Running => SummaryBootstrapStatus::Running,
        DevqlTaskStatus::Completed => SummaryBootstrapStatus::Completed,
        DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled => SummaryBootstrapStatus::Failed,
    }
}

pub(crate) fn task_status_is_terminal(status: DevqlTaskStatus) -> bool {
    matches!(
        status,
        DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
    )
}

pub(crate) fn task_status_is_failed(status: DevqlTaskStatus) -> bool {
    matches!(status, DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled)
}

pub(crate) fn task_status_is_completed(status: DevqlTaskStatus) -> bool {
    status == DevqlTaskStatus::Completed
}

pub(crate) fn summary_status_is_terminal(status: SummaryBootstrapStatus) -> bool {
    matches!(
        status,
        SummaryBootstrapStatus::Completed | SummaryBootstrapStatus::Failed
    )
}

pub(crate) fn summary_status_is_failed(status: SummaryBootstrapStatus) -> bool {
    status == SummaryBootstrapStatus::Failed
}

pub(crate) fn summary_status_is_completed(status: SummaryBootstrapStatus) -> bool {
    status == SummaryBootstrapStatus::Completed
}

pub(crate) fn effective_task_id(
    task: Option<&DevqlTaskRecord>,
    terminal: Option<&InitSessionTaskTerminalSnapshot>,
    fallback: Option<&str>,
) -> Option<String> {
    task.map(|task| task.task_id.clone())
        .or_else(|| terminal.map(|terminal| terminal.task_id.clone()))
        .or_else(|| fallback.map(str::to_string))
}

pub(crate) fn initial_sync_status(
    session: &InitSessionRecord,
    task: Option<&DevqlTaskRecord>,
) -> Option<DevqlTaskStatus> {
    effective_task_status(task, session.initial_sync_terminal.as_ref()).or_else(|| {
        session
            .initial_sync_completion_seq
            .map(|_| DevqlTaskStatus::Completed)
    })
}

pub(crate) fn ingest_status(
    session: &InitSessionRecord,
    task: Option<&DevqlTaskRecord>,
) -> Option<DevqlTaskStatus> {
    effective_task_status(task, session.ingest_terminal.as_ref())
}

pub(crate) fn embeddings_bootstrap_status(
    session: &InitSessionRecord,
    task: Option<&DevqlTaskRecord>,
) -> Option<DevqlTaskStatus> {
    effective_task_status(task, session.embeddings_bootstrap_terminal.as_ref()).or_else(|| {
        session
            .embeddings_bootstrap_completion_seq
            .map(|_| DevqlTaskStatus::Completed)
    })
}

pub(crate) fn follow_up_sync_status(
    session: &InitSessionRecord,
    task: Option<&DevqlTaskRecord>,
) -> Option<DevqlTaskStatus> {
    effective_task_status(task, session.follow_up_sync_terminal.as_ref()).or_else(|| {
        session
            .follow_up_sync_completion_seq
            .map(|_| DevqlTaskStatus::Completed)
    })
}

pub(crate) fn summary_bootstrap_status(
    session: &InitSessionRecord,
    run: Option<&SummaryBootstrapRunRecord>,
) -> Option<SummaryBootstrapStatus> {
    run.map(|run| run.status)
        .or_else(|| {
            session
                .summary_bootstrap_terminal
                .as_ref()
                .map(|terminal| summary_status_from_task_status(terminal.status))
        })
        .or_else(|| {
            session
                .summary_bootstrap_completion_seq
                .map(|_| SummaryBootstrapStatus::Completed)
        })
}

fn effective_task_status(
    task: Option<&DevqlTaskRecord>,
    terminal: Option<&InitSessionTaskTerminalSnapshot>,
) -> Option<DevqlTaskStatus> {
    task.map(|task| task.status)
        .or_else(|| terminal.map(|terminal| terminal.status))
}
