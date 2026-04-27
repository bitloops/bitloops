use anyhow::{Context, Result, anyhow};

use crate::config::resolve_store_backend_config_for_repo;
use crate::host::devql::{DevqlConfig, RelationalStorage};

use super::super::{
    checkpoint_commit_info_from_sha, ensure_repository_row, upsert_checkpoint_file_snapshot_rows,
};

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

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-commit checkpoint projection refresh")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "git post-commit checkpoint projection refresh",
    )
    .await?;

    refresh_checkpoint_projection_for_commit(cfg, &relational, commit_sha, checkpoint_id).await
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
    .ok_or_else(|| anyhow!("checkpoint not found for projection refresh: {checkpoint_id}"))?;
    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, commit_sha);

    let _projected_rows = upsert_checkpoint_file_snapshot_rows(
        cfg,
        relational,
        &checkpoint,
        commit_sha,
        commit_info.as_ref(),
    )
    .await?;

    Ok(())
}
