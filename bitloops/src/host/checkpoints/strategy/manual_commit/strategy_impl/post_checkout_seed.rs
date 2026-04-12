use super::*;

pub(crate) fn run_devql_post_checkout_seed(
    repo_root: &Path,
    previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout
        || new_head.trim().is_empty()
        || new_head.trim().chars().all(|ch| ch == '0')
    {
        return Ok(());
    }

    #[cfg(not(test))]
    {
        let _ = previous_head;
        crate::host::devql::enqueue_spooled_sync_task_for_repo_root(
            repo_root,
            crate::daemon::DevqlTaskSource::PostCheckout,
            crate::host::devql::SyncMode::Full,
        )
        .context("queueing full DevQL sync for post-checkout branch seed in repo-local spool")?;
        Ok(())
    }

    #[cfg(test)]
    {
        let seed_future = async {
            let repo = crate::host::devql::resolve_repo_identity(repo_root)
                .context("resolving repository identity for post-checkout DevQL seeding")?;
            let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
                .context("building DevQL config for post-checkout seeding")?;
            crate::host::devql::run_post_checkout_branch_seed(
                &cfg,
                previous_head,
                new_head,
                is_branch_checkout,
            )
            .await
            .context("seeding DevQL artefacts for post-checkout branch switch")?;

            Ok::<(), anyhow::Error>(())
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return tokio::task::block_in_place(|| handle.block_on(seed_future));
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building tokio runtime for post-checkout DevQL seeding")?;
        runtime.block_on(seed_future)
    }
}
