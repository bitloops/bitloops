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

    let pg_dsn = remote_dsn.unwrap_or_default();
    let mut pg_cfg: tokio_postgres::Config = pg_dsn.parse().context("parsing Postgres DSN")?;
    pg_cfg.connect_timeout(std::time::Duration::from_secs(10));
    let connect_result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        pg_cfg.connect(tokio_postgres::NoTls),
    )
    .await
    .context("Postgres connect timeout after 10s");
    let (client, connection) = match connect_result {
        Ok(Ok(pair)) => pair,
        Ok(Err(err)) => {
            for update in &updates {
                let _ = sync_state::mark_branch_sync_pending(
                    &local,
                    &repo.repo_id,
                    remote_name,
                    &update.remote_branch,
                    &update.local_sha,
                )
                .await;
            }
            return Err(err).context("connecting to Postgres for pre-push DevQL sync");
        }
        Err(err) => {
            for update in &updates {
                let _ = sync_state::mark_branch_sync_pending(
                    &local,
                    &repo.repo_id,
                    remote_name,
                    &update.remote_branch,
                    &update.local_sha,
                )
                .await;
            }
            return Err(err);
        }
    };

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::warn!("Postgres connection task ended during pre-push DevQL sync: {err:#}");
        }
    });

    let relational = local_store.with_remote_client(client);
    let mut synced_heads: Vec<String> = Vec::new();
    for update in &updates {
        sync_state::mark_branch_sync_pending(
            &local,
            &repo.repo_id,
            remote_name,
            &update.remote_branch,
            &update.local_sha,
        )
        .await?;

        let commits_to_sync = commit_selection::list_commits_to_sync_for_ref_update(
            repo_root,
            &local,
            &repo.repo_id,
            remote_name,
            update,
        )
        .await
        .with_context(|| {
            format!(
                "resolving commits to sync for `{}` -> `{}`",
                update.local_ref, update.remote_ref
            )
        })?;

        for commit_sha in commits_to_sync {
            history_replication::replicate_history_for_commit(
                &relational,
                &repo.repo_id,
                &commit_sha,
            )
            .await
            .with_context(|| {
                format!(
                    "replicating DevQL historical rows for commit {} to remote branch `{}`",
                    commit_sha, update.remote_branch
                )
            })?;
        }

        let source_branch = update
            .local_branch
            .as_deref()
            .unwrap_or(update.remote_branch.as_str());
        current_state_replication::sync_remote_branch_current_state(
            &relational,
            &repo.repo_id,
            source_branch,
            &update.remote_branch,
        )
        .await
        .with_context(|| {
            format!(
                "syncing DevQL current-state rows from local branch `{source_branch}` to remote branch `{}`",
                update.remote_branch
            )
        })?;

        sync_state::mark_branch_sync_complete(
            &local,
            &repo.repo_id,
            remote_name,
            &update.remote_branch,
            &update.local_sha,
        )
        .await?;
        synced_heads.push(update.local_sha.clone());
    }

    for commit_sha in synced_heads {
        pruning::prune_historical_rows_up_to_commit(&local, &repo.repo_id, &commit_sha)
            .await
            .with_context(|| {
                format!("pruning local historical DevQL rows after syncing commit {commit_sha}")
            })?;
    }

    Ok(())
}
