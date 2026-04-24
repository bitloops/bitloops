use anyhow::{Context, Result};

use crate::host::devql::{DevqlConfig, SyncMode};

pub async fn run_post_checkout_branch_seed(
    cfg: &DevqlConfig,
    _previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout || new_head.trim().is_empty() || is_zero_git_oid(new_head) {
        return Ok(());
    }

    #[cfg(test)]
    {
        crate::host::devql::run_sync_with_summary(cfg, SyncMode::Full)
            .await
            .context("running full DevQL sync inline for post-checkout branch seed in tests")?;
        Ok(())
    }

    #[cfg(not(test))]
    {
        crate::daemon::enqueue_sync_for_config(
            cfg,
            crate::daemon::DevqlTaskSource::PostCheckout,
            SyncMode::Full,
        )
        .context("queueing full DevQL sync for post-checkout branch seed")?;
        Ok(())
    }
}

fn is_zero_git_oid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '0')
}
