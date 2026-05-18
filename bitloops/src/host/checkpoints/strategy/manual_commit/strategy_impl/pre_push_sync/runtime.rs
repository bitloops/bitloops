use super::*;

pub(crate) fn run_devql_pre_push_sync(
    repo_root: &Path,
    remote: &str,
    stdin_lines: &[String],
) -> Result<()> {
    if !crate::config::settings::devql_sync_enabled(repo_root)
        .context("loading DevQL sync producer policy for pre-push sync")?
    {
        return Ok(());
    }

    #[cfg(not(test))]
    {
        crate::host::devql::enqueue_spooled_pre_push_sync(repo_root, remote, stdin_lines)
            .context("queueing pre-push DevQL sync in repo-local spool")?;
        Ok(())
    }

    #[cfg(test)]
    {
        let repo_root = repo_root.to_path_buf();
        let remote = remote.trim().to_string();
        let stdin_lines = stdin_lines.to_vec();
        let sync_future =
            async move { execute_devql_pre_push_sync(&repo_root, &remote, &stdin_lines).await };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            return tokio::task::block_in_place(|| handle.block_on(sync_future));
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building tokio runtime for pre-push DevQL sync")?;
        runtime.block_on(sync_future)
    }
}

pub(crate) async fn execute_devql_pre_push_sync(
    repo_root: &Path,
    remote: &str,
    stdin_lines: &[String],
) -> Result<()> {
    if !crate::config::settings::devql_sync_enabled(repo_root)
        .context("loading DevQL sync producer policy for pre-push sync")?
    {
        return Ok(());
    }

    let repo = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repository identity for pre-push DevQL sync")?;
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving backend config for pre-push DevQL sync")?;
    let local_store =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening local relational store for pre-push DevQL sync")?;
    let sqlite_path = local_store.sqlite_path().to_path_buf();
    if !sqlite_path.exists() {
        return Ok(());
    }

    let local = local_store.to_local_inner();

    let remote_dsn = backends
        .relational
        .postgres_dsn
        .as_deref()
        .map(str::trim)
        .filter(|dsn| !dsn.is_empty())
        .map(str::to_string);

    if remote_dsn.is_none() {
        pruning::prune_historical_rows_with_retention(
            &local,
            &repo.repo_id,
            constants::PRE_PUSH_RETENTION_COMMITS,
        )
        .await
        .context("pruning local historical DevQL rows with retention policy")?;
        return Ok(());
    }

    let updates = parsing::collect_pre_push_ref_updates(stdin_lines);
    if updates.is_empty() {
        return Ok(());
    }

    let remote_name = if remote.trim().is_empty() {
        "origin"
    } else {
        remote.trim()
    };

    if let Some(dsn) = remote_dsn.as_deref()
        && let Err(err) = crate::storage::postgres::connect_postgres_client(dsn).await
    {
        record_remote_branch_sync_pending(&local, &repo.repo_id, remote_name, &updates).await?;
        return Err(err).context("connecting shared relational Postgres for pre-push DevQL sync");
    }

    record_remote_branch_sync_watermarks(&local, &repo.repo_id, remote_name, &updates).await
}

pub(super) async fn record_remote_branch_sync_watermarks(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    updates: &[types::PrePushRefUpdate],
) -> Result<()> {
    for update in updates {
        sync_state::mark_branch_sync_complete(
            local,
            repo_id,
            remote_name,
            &update.remote_branch,
            &update.local_sha,
        )
        .await?;
    }
    Ok(())
}

async fn record_remote_branch_sync_pending(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    updates: &[types::PrePushRefUpdate],
) -> Result<()> {
    for update in updates {
        sync_state::mark_branch_sync_pending(
            local,
            repo_id,
            remote_name,
            &update.remote_branch,
            &update.local_sha,
        )
        .await?;
    }
    Ok(())
}
