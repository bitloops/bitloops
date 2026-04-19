use anyhow::Result;

use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskStatus, SummaryBootstrapRunRecord, SummaryBootstrapStatus,
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
