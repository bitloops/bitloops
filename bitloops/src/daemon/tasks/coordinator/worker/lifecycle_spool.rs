use anyhow::{Context, Result};

use crate::storage::SqliteConnectionPool;

pub(crate) fn recover_lifecycle_spool_jobs(sqlite: &SqliteConnectionPool) -> Result<u64> {
    crate::host::checkpoints::lifecycle::spool::recover_running_lifecycle_jobs(sqlite)
}

pub(crate) fn process_lifecycle_spool_once(sqlite: &SqliteConnectionPool) -> Result<u64> {
    let Some(job) = crate::host::checkpoints::lifecycle::spool::claim_next_lifecycle_job(sqlite)?
    else {
        return Ok(0);
    };

    match process_lifecycle_job(&job) {
        Ok(()) => {
            crate::host::checkpoints::lifecycle::spool::delete_lifecycle_job(sqlite, &job.job_id)?;
            Ok(1)
        }
        Err(err) => {
            log::warn!(
                "failed to process lifecycle spool job {} for agent={} hook={} repo={}: {err:#}",
                job.job_id,
                job.agent_name,
                job.hook_name,
                job.repo_root.display()
            );
            crate::host::checkpoints::lifecycle::spool::fail_or_requeue_lifecycle_job(
                sqlite,
                &job,
                &format!("{err:#}"),
            )?;
            Ok(1)
        }
    }
}

fn process_lifecycle_job(
    job: &crate::host::checkpoints::lifecycle::spool::LifecycleJobRecord,
) -> Result<()> {
    crate::host::checkpoints::lifecycle::adapters::route_hook_command_to_lifecycle_for_repo_with_boundary_snapshot(
        &job.repo_root,
        &job.agent_name,
        &job.hook_name,
        &job.raw_stdin,
        job.boundary_snapshot.clone(),
    )
    .with_context(|| {
        format!(
            "processing lifecycle spool job `{}` for agent={} hook={} repo={}",
            job.job_id,
            job.agent_name,
            job.hook_name,
            job.repo_root.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn process_lifecycle_spool_once_for_tests(sqlite: &SqliteConnectionPool) -> Result<u64> {
    process_lifecycle_spool_once(sqlite)
}
