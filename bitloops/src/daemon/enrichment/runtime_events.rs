use anyhow::Result;

use crate::daemon::types::unix_timestamp_now;

use super::EnrichmentJobTarget;

pub(crate) fn publish_workplane_runtime_event(
    target: &EnrichmentJobTarget,
    mailbox_name: &str,
) -> Result<()> {
    let Some(init_session_id) = target.init_session_id.clone() else {
        return Ok(());
    };
    let repo_id = match target.repo_id.as_ref() {
        Some(repo_id) => repo_id.clone(),
        None => crate::host::devql::resolve_repo_identity(&target.repo_root)?.repo_id,
    };
    crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
        crate::daemon::RuntimeEventRecord {
            domain: "workplane".to_string(),
            repo_id,
            init_session_id: Some(init_session_id),
            updated_at_unix: unix_timestamp_now(),
            task_id: None,
            run_id: None,
            mailbox_name: Some(mailbox_name.to_string()),
        },
    );
    Ok(())
}

pub(crate) fn publish_job_runtime_event(job: &crate::host::runtime_store::WorkplaneJobRecord) {
    let Some(init_session_id) = job.init_session_id.clone() else {
        return;
    };
    crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
        crate::daemon::RuntimeEventRecord {
            domain: "workplane".to_string(),
            repo_id: job.repo_id.clone(),
            init_session_id: Some(init_session_id),
            updated_at_unix: unix_timestamp_now(),
            task_id: None,
            run_id: None,
            mailbox_name: Some(job.mailbox_name.clone()),
        },
    );
}
