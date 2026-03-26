use super::*;

pub(crate) fn run_devql_post_commit_refresh(
    repo_root: &Path,
    commit_sha: &str,
    committed_files: &std::collections::HashSet<String>,
) -> Result<()> {
    let mut changed_files = committed_files.iter().cloned().collect::<Vec<_>>();
    changed_files.sort();

    run_post_commit_future(repo_root, async {
        let repo = crate::host::devql::resolve_repo_identity(repo_root)
            .context("resolving repository identity for post-commit DevQL refresh")?;
        let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
            .context("building DevQL config for post-commit refresh")?;
        let stats =
            crate::host::devql::run_post_commit_artefact_refresh(&cfg, commit_sha, &changed_files)
                .await
                .context("refreshing DevQL artefacts for post-commit files")?;

        if stats.files_failed > 0 {
            eprintln!(
                "[bitloops] Warning: DevQL post-commit artefact refresh partially succeeded for commit {} (seen={}, indexed={}, deleted={}, failed={})",
                commit_sha,
                stats.files_seen,
                stats.files_indexed,
                stats.files_deleted,
                stats.files_failed
            );
        }
        Ok::<(), anyhow::Error>(())
    })
}

pub(crate) fn run_devql_post_commit_checkpoint_projection_refresh(
    repo_root: &Path,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
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
