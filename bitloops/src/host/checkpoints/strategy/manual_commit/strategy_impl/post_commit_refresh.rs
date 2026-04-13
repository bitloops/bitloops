use super::*;

const DEFAULT_POST_COMMIT_HISTORY_BACKFILL: usize = 50;
const DISABLE_POST_COMMIT_DEVQL_REFRESH_ENV: &str = "BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH";

fn should_skip_post_commit_devql_refresh() -> bool {
    std::env::var(DISABLE_POST_COMMIT_DEVQL_REFRESH_ENV).is_ok_and(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && trimmed != "0"
    })
}

pub(crate) fn run_devql_post_commit_refresh(
    repo_root: &Path,
    commit_sha: &str,
    committed_files: &std::collections::HashSet<String>,
) -> Result<()> {
    if should_skip_post_commit_devql_refresh() {
        return Ok(());
    }

    let mut changed_files = committed_files.iter().cloned().collect::<Vec<_>>();
    changed_files.sort();

    #[cfg(not(test))]
    {
        crate::host::devql::enqueue_spooled_post_commit_refresh(
            repo_root,
            commit_sha,
            &changed_files,
        )
        .context("queueing post-commit DevQL refresh in repo-local spool")?;
        Ok(())
    }

    #[cfg(test)]
    {
        run_post_commit_future(repo_root, async {
            let repo = crate::host::devql::resolve_repo_identity(repo_root)
                .context("resolving repository identity for post-commit DevQL refresh")?;
            let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
                .context("building DevQL config for post-commit refresh")?;
            execute_devql_post_commit_refresh(&cfg, commit_sha, &changed_files).await
        })
    }
}

pub(crate) async fn execute_devql_post_commit_refresh(
    cfg: &crate::host::devql::DevqlConfig,
    commit_sha: &str,
    changed_files: &[String],
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    if commit_sha.is_empty() {
        return Ok(());
    }

    crate::host::devql::execute_ingest_with_backfill_window(
        cfg,
        false,
        DEFAULT_POST_COMMIT_HISTORY_BACKFILL,
        None,
        None,
    )
    .await
    .context("catching up DevQL historical commit ingest for post-commit")?;
    let stats =
        crate::host::devql::run_post_commit_artefact_refresh(cfg, commit_sha, changed_files)
            .await
            .context("refreshing DevQL artefacts for post-commit files")?;

    if stats.completed_with_failures() {
        log::warn!(
            "DevQL post-commit artefact refresh partially succeeded for commit {} (seen={}, indexed={}, deleted={}, failed={})",
            commit_sha,
            stats.files_seen,
            stats.files_indexed,
            stats.files_deleted,
            stats.files_failed
        );
    }

    Ok(())
}

pub(crate) fn run_devql_post_commit_checkpoint_projection_refresh(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    if should_skip_post_commit_devql_refresh() {
        return Ok(());
    }

    run_post_commit_future(repo_root, async {
        let repo = crate::host::devql::resolve_repo_identity(repo_root).context(
            "resolving repository identity for post-commit checkpoint projection refresh",
        )?;
        let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
            .context("building DevQL config for post-commit checkpoint projection refresh")?;
        crate::host::devql::run_post_commit_checkpoint_projection_refresh(
            &cfg,
            commit_sha,
            checkpoint_id,
        )
        .await
        .context("refreshing DevQL checkpoint projection after post-commit")?;
        Ok::<(), anyhow::Error>(())
    })
}

fn run_post_commit_future<F>(_repo_root: &Path, refresh_future: F) -> Result<()>
where
    F: std::future::Future<Output = Result<()>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(refresh_future));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for post-commit DevQL work")?;
    runtime.block_on(refresh_future)
}
