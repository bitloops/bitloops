use super::*;

pub(crate) fn run_devql_post_commit_refresh(
    repo_root: &Path,
    commit_sha: &str,
    committed_files: &std::collections::HashSet<String>,
) -> Result<()> {
    let mut changed_files = committed_files.iter().cloned().collect::<Vec<_>>();
    changed_files.sort();

    let refresh_future = async {
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
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(refresh_future));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for post-commit DevQL refresh")?;
    runtime.block_on(refresh_future)
}
