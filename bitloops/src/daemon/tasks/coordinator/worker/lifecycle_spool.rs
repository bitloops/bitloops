use anyhow::{Context, Result};

use crate::storage::SqliteConnectionPool;

const LIFECYCLE_STOP_CLAIM_LIMIT: usize = 8;

pub(crate) fn recover_lifecycle_stop_spool_jobs(sqlite: &SqliteConnectionPool) -> Result<u64> {
    crate::host::checkpoints::lifecycle::spool::recover_running_lifecycle_stop_jobs(sqlite)
}

pub(crate) fn process_lifecycle_stop_spool_once(sqlite: &SqliteConnectionPool) -> Result<u64> {
    let jobs = crate::host::checkpoints::lifecycle::spool::claim_next_lifecycle_stop_jobs(
        sqlite,
        LIFECYCLE_STOP_CLAIM_LIMIT,
    )?;
    let mut processed = 0u64;
    for job in jobs {
        match process_lifecycle_stop_job(&job) {
            Ok(()) => {
                crate::host::checkpoints::lifecycle::spool::delete_lifecycle_stop_job(
                    sqlite,
                    &job.job_id,
                )?;
                processed += 1;
            }
            Err(err) => {
                log::warn!(
                    "failed to process lifecycle stop spool job {} for agent={} repo={}: {err:#}",
                    job.job_id,
                    job.agent_name,
                    job.repo_root.display()
                );
                crate::host::checkpoints::lifecycle::spool::requeue_lifecycle_stop_job(
                    sqlite,
                    &job.job_id,
                    &format!("{err:#}"),
                )?;
            }
        }
    }
    Ok(processed)
}

fn process_lifecycle_stop_job(
    job: &crate::host::checkpoints::lifecycle::spool::LifecycleStopJobRecord,
) -> Result<()> {
    crate::host::checkpoints::lifecycle::adapters::route_hook_command_to_lifecycle_for_repo(
        &job.repo_root,
        &job.agent_name,
        &job.hook_name,
        &job.raw_stdin,
    )
    .with_context(|| {
        format!(
            "processing lifecycle stop spool job `{}` for agent={} repo={}",
            job.job_id,
            job.agent_name,
            job.repo_root.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn process_lifecycle_stop_spool_once_for_tests(
    sqlite: &SqliteConnectionPool,
) -> Result<u64> {
    process_lifecycle_stop_spool_once(sqlite)
}
