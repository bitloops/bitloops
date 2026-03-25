use super::*;

pub(super) async fn list_commits_to_sync_for_ref_update(
    repo_root: &Path,
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    update: &types::PrePushRefUpdate,
) -> Result<Vec<String>> {
    let watermark_key = sync_state::branch_sync_watermark_key(remote_name, &update.remote_branch);
    let last_synced = sync_state::load_sync_state_value(local, repo_id, &watermark_key).await?;

    if let Some(last_synced_sha) = last_synced.as_deref()
        && !last_synced_sha.trim().is_empty()
        && last_synced_sha != update.local_sha
        && git_is_ancestor(repo_root, last_synced_sha, &update.local_sha)
    {
        let range = format!("{last_synced_sha}..{}", update.local_sha);
        let commits = list_commit_range(repo_root, &range)?;
        if !commits.is_empty() {
            return Ok(commits);
        }
    }

    if parsing::is_zero_git_oid(&update.remote_sha) {
        let candidate = run_git(
            repo_root,
            &[
                "rev-list",
                "--reverse",
                &update.local_sha,
                "--not",
                "--remotes",
            ],
        )
        .unwrap_or_default();
        let mut commits = parsing::parse_sha_lines(&candidate);
        if commits.is_empty() {
            commits.push(update.local_sha.clone());
        }
        return Ok(commits);
    }

    let range = format!("{}..{}", update.remote_sha, update.local_sha);
    let commits = match list_commit_range(repo_root, &range) {
        Ok(commits) => commits,
        Err(_) => vec![update.local_sha.clone()],
    };
    Ok(commits)
}

fn list_commit_range(repo_root: &Path, range: &str) -> Result<Vec<String>> {
    let output = run_git(repo_root, &["rev-list", "--reverse", range])?;
    Ok(parsing::parse_sha_lines(&output))
}

fn git_is_ancestor(repo_root: &Path, ancestor: &str, descendant: &str) -> bool {
    run_git(
        repo_root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )
    .is_ok()
}
