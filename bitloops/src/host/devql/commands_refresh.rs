use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub struct PostCommitArtefactRefreshStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub files_failed: usize,
}

pub async fn run_post_commit_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-commit").await
}

pub async fn run_post_commit_checkpoint_projection_refresh(
    cfg: &DevqlConfig,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    let checkpoint_id = checkpoint_id.trim();
    if commit_sha.is_empty() || checkpoint_id.is_empty() {
        return Ok(());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
        .context("resolving DevQL backend config for post-commit checkpoint projection refresh")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "git post-commit checkpoint projection refresh",
    )
    .await?;

    refresh_checkpoint_projection_for_commit(cfg, &relational, commit_sha, checkpoint_id).await
}

pub async fn run_post_merge_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-merge").await
}

async fn sync_changed_paths(
    cfg: &DevqlConfig,
    changed_files: &[String],
    source_hook: &str,
) -> Result<PostCommitArtefactRefreshStats> {
    let mut paths = changed_files
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let mut stats = PostCommitArtefactRefreshStats {
        files_seen: paths.len(),
        ..PostCommitArtefactRefreshStats::default()
    };
    if paths.is_empty() {
        return Ok(stats);
    }

    let summary = run_sync_with_summary(cfg, SyncMode::Paths(paths))
        .await
        .with_context(|| format!("running DevQL sync for {source_hook} refresh"))?;
    stats.files_indexed = summary.paths_added + summary.paths_changed;
    stats.files_deleted = summary.paths_removed;
    stats.files_failed = summary.parse_errors;
    Ok(stats)
}

async fn refresh_checkpoint_projection_for_commit(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    ensure_repository_row(cfg, relational).await?;

    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
        &cfg.repo_root,
        checkpoint_id,
    )?
    .ok_or_else(|| anyhow::anyhow!("checkpoint not found for projection refresh: {checkpoint_id}"))?;
    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, commit_sha);

    let _projected_rows =
        upsert_checkpoint_file_snapshot_rows(cfg, relational, &checkpoint, commit_sha, commit_info.as_ref())
            .await?;

    Ok(())
}

pub async fn run_post_checkout_branch_seed(
    cfg: &DevqlConfig,
    _previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout || new_head.trim().is_empty() || is_zero_git_oid(new_head) {
        return Ok(());
    }

    run_sync_with_summary(cfg, SyncMode::Full)
        .await
        .context("running full DevQL sync for post-checkout branch seed")?;
    Ok(())
}

fn is_zero_git_oid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '0')
}
