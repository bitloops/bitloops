#[path = "tasks/coordinator.rs"]
mod coordinator;
#[path = "tasks/queue.rs"]
mod queue;
#[path = "tasks/state.rs"]
mod state;

pub use self::coordinator::{DevqlTaskCoordinator, DevqlTaskEnqueueResult};

#[cfg(feature = "slow-tests")]
pub fn drain_lifecycle_stop_spool_for_repo_for_tests(
    repo_root: &std::path::Path,
) -> anyhow::Result<u64> {
    let config_root = crate::config::resolve_bound_daemon_config_root_for_repo(repo_root)?;
    let sqlite = crate::host::runtime_store::open_runtime_sqlite_for_config_root(&config_root)?;
    coordinator::recover_lifecycle_stop_spool_jobs_for_tests(&sqlite)?;

    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)?.repo_id;
    let mut processed_total = 0;
    for _ in 0..16 {
        let processed = coordinator::process_lifecycle_stop_spool_once_for_tests(&sqlite)?;
        processed_total += processed;
        if !crate::host::checkpoints::lifecycle::spool::lifecycle_stop_spool_has_repo_work(
            &sqlite, &repo_id,
        )? {
            return Ok(processed_total);
        }
        if processed == 0 {
            break;
        }
    }

    anyhow::bail!(
        "lifecycle stop spool still has pending work for repo {} after draining",
        repo_root.display()
    )
}
