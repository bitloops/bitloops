use super::*;

#[derive(Debug, Clone, Default)]
pub(crate) struct BranchDeletionTargets {
    pub(super) local_branches: Vec<String>,
    pub(super) remote_branches: Vec<String>,
}

const ZERO_GIT_OID: &str = "0000000000000000000000000000000000000000";

pub(crate) fn collect_reference_transaction_branch_deletions(
    state: &str,
    stdin_lines: &[String],
) -> BranchDeletionTargets {
    if !state.eq_ignore_ascii_case("committed") {
        return BranchDeletionTargets::default();
    }

    let mut local = std::collections::BTreeSet::new();
    let mut remote = std::collections::BTreeSet::new();

    for line in stdin_lines {
        let Some((_, new_sha, ref_name)) = parse_reference_transaction_update_line(line) else {
            continue;
        };
        if !is_zero_git_oid(new_sha) {
            continue;
        }

        if let Some(branch_name) = ref_name.strip_prefix("refs/heads/")
            && !branch_name.trim().is_empty()
        {
            local.insert(branch_name.to_string());
            continue;
        }

        if let Some(branch_name) = ref_name.strip_prefix("refs/remotes/")
            && !branch_name.trim().is_empty()
        {
            remote.insert(branch_name.to_string());
        }
    }

    BranchDeletionTargets {
        local_branches: local.into_iter().collect(),
        remote_branches: remote.into_iter().collect(),
    }
}

fn parse_reference_transaction_update_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.split_whitespace();
    let old_sha = parts.next()?;
    let new_sha = parts.next()?;
    let ref_name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((old_sha, new_sha, ref_name))
}

fn is_zero_git_oid(value: &str) -> bool {
    value.trim() == ZERO_GIT_OID
}

pub(crate) fn run_devql_reference_transaction_cleanup(
    repo_root: &Path,
    deletions: &BranchDeletionTargets,
) -> Result<()> {
    let deletions = deletions.clone();
    let repo_root = repo_root.to_path_buf();
    let cleanup_future = async move {
        let repo = crate::host::devql::resolve_repo_identity(&repo_root)
            .context("resolving repository identity for reference-transaction cleanup")?;
        let backends = crate::config::resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for reference-transaction cleanup")?;
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(&repo_root)
            .context("resolving SQLite path for reference-transaction cleanup")?;

        if !deletions.local_branches.is_empty() && sqlite_path.exists() {
            let sqlite = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());
            let statements =
                build_current_state_cleanup_sql(&repo.repo_id, &deletions.local_branches);
            if let Err(err) = sqlite.exec_batch_transactional(&statements).await
                && !is_missing_devql_current_state_schema_error(&err)
            {
                return Err(err).context(
                    "cleaning local branch current-state rows in SQLite for reference-transaction",
                );
            }
        }

        if !deletions.remote_branches.is_empty()
            && let Some(pg_dsn) = backends.relational.postgres_dsn.as_deref()
            && !pg_dsn.trim().is_empty()
        {
            let mut pg_cfg: tokio_postgres::Config =
                pg_dsn.parse().context("parsing Postgres DSN")?;
            pg_cfg.connect_timeout(std::time::Duration::from_secs(10));
            let (client, connection) = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                pg_cfg.connect(tokio_postgres::NoTls),
            )
            .await
            .context("Postgres connect timeout after 10s")?
            .context("connecting to Postgres for reference-transaction cleanup")?;
            tokio::spawn(async move {
                if let Err(err) = connection.await {
                    log::warn!(
                        "Postgres connection task ended during reference-transaction cleanup: {err:#}"
                    );
                }
            });

            let postgres =
                crate::host::devql::RelationalStorage::with_remote_client(sqlite_path, client);
            let statements =
                build_current_state_cleanup_sql(&repo.repo_id, &deletions.remote_branches);
            if let Err(err) = postgres.exec_remote_batch_transactional(&statements).await
                && !is_missing_devql_current_state_schema_error(&err)
            {
                return Err(err).context(
                    "cleaning remote branch current-state rows in Postgres for reference-transaction",
                );
            }
        }

        Ok::<(), anyhow::Error>(())
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(cleanup_future));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for reference-transaction cleanup")?;
    runtime.block_on(cleanup_future)
}

fn build_current_state_cleanup_sql(repo_id: &str, branches: &[String]) -> Vec<String> {
    let mut statements = Vec::with_capacity(branches.len() * 2);
    for branch in branches {
        statements.push(format!(
            "DELETE FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(branch),
        ));
        statements.push(format!(
            "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(branch),
        ));
    }
    statements
}

fn is_missing_devql_current_state_schema_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    let references_current_state = msg.contains("artefacts_current")
        || msg.contains("artefact_edges_current")
        || msg.contains("current-state");
    references_current_state && (msg.contains("no such table") || msg.contains("does not exist"))
}
