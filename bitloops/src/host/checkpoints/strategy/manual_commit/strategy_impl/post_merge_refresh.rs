use super::*;

const DEFAULT_POST_MERGE_HISTORY_BACKFILL: usize = 200;

pub(crate) fn run_devql_post_merge_refresh(repo_root: &Path, _is_squash: bool) -> Result<()> {
    let Some(head_sha) = try_head_hash(repo_root)? else {
        return Ok(());
    };

    let changed_files = match files_changed_since_previous_head(repo_root) {
        Ok(files) => files,
        Err(err) if is_missing_previous_head_error(&err) => Vec::new(),
        Err(err) => {
            return Err(err).context("listing changed files between HEAD@{1} and HEAD");
        }
    };
    if changed_files.is_empty() {
        return Ok(());
    }

    #[cfg(not(test))]
    {
        crate::host::devql::enqueue_spooled_post_merge_refresh(
            repo_root,
            &head_sha,
            &changed_files,
        )
        .context("queueing post-merge DevQL refresh in repo-local spool")?;
        Ok(())
    }

    #[cfg(test)]
    {
        let refresh_future = async {
            let repo = crate::host::devql::resolve_repo_identity(repo_root)
                .context("resolving repository identity for post-merge DevQL refresh")?;
            let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.to_path_buf(), repo)
                .context("building DevQL config for post-merge refresh")?;
            execute_devql_post_merge_refresh(&cfg, &head_sha, &changed_files).await
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return tokio::task::block_in_place(|| handle.block_on(refresh_future));
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building tokio runtime for post-merge DevQL refresh")?;
        runtime.block_on(refresh_future)
    }
}

pub(crate) async fn execute_devql_post_merge_refresh(
    cfg: &crate::host::devql::DevqlConfig,
    head_sha: &str,
    changed_files: &[String],
) -> Result<()> {
    let head_sha = head_sha.trim();
    if head_sha.is_empty() {
        return Ok(());
    }

    crate::host::devql::execute_ingest_with_backfill_window(
        cfg,
        false,
        DEFAULT_POST_MERGE_HISTORY_BACKFILL,
        None,
        None,
    )
    .await
    .context("catching up DevQL historical commit ingest for post-merge")?;
    let stats = crate::host::devql::run_post_merge_artefact_refresh(cfg, head_sha, changed_files)
        .await
        .context("refreshing DevQL artefacts for post-merge files")?;

    if stats.completed_with_failures() {
        log::warn!(
            "DevQL post-merge artefact refresh partially succeeded for commit {} (seen={}, indexed={}, deleted={}, failed={})",
            head_sha,
            stats.files_seen,
            stats.files_indexed,
            stats.files_deleted,
            stats.files_failed
        );
    }

    Ok(())
}

fn files_changed_since_previous_head(repo_root: &Path) -> Result<Vec<String>> {
    let mut changed_files = run_git(repo_root, &["diff", "--name-only", "HEAD@{1}", "HEAD"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    changed_files.sort();
    changed_files.dedup();
    Ok(changed_files)
}

fn is_missing_previous_head_error(err: &anyhow::Error) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("head@{1}")
        && (message.contains("unknown revision")
            || message.contains("ambiguous argument")
            || message.contains("fatal"))
}
