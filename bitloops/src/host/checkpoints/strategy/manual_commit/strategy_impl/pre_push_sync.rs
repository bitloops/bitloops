use super::*;

const ZERO_GIT_OID: &str = "0000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrePushRefUpdate {
    local_ref: String,
    local_sha: String,
    remote_ref: String,
    remote_sha: String,
    local_branch: Option<String>,
    remote_branch: String,
}

const PRE_PUSH_RETENTION_COMMITS: usize = 50;
const PRE_PUSH_SYNC_WATERMARK_KEY: &str = "last_synced_commit_sha";
const PRE_PUSH_SYNC_PENDING_KEY_PREFIX: &str = "pending_remote_sync_sha";
const PRE_PUSH_BATCH_SIZE: usize = 200;

pub(crate) fn run_devql_pre_push_sync(
    repo_root: &Path,
    remote: &str,
    stdin_lines: &[String],
) -> Result<()> {
    let repo_root = repo_root.to_path_buf();
    let remote = remote.trim().to_string();
    let stdin_lines = stdin_lines.to_vec();

    let sync_future = async move {
        let repo = crate::host::devql::resolve_repo_identity(&repo_root)
            .context("resolving repository identity for pre-push DevQL sync")?;
        let backends = crate::config::resolve_store_backend_config_for_repo(&repo_root)
            .context("resolving backend config for pre-push DevQL sync")?;
        let sqlite_path = backends
            .relational
            .resolve_sqlite_db_path_for_repo(&repo_root)
            .context("resolving SQLite path for pre-push DevQL sync")?;
        if !sqlite_path.exists() {
            return Ok::<(), anyhow::Error>(());
        }

        let local = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());

        let remote_dsn = backends
            .relational
            .postgres_dsn
            .as_deref()
            .map(str::trim)
            .filter(|dsn| !dsn.is_empty())
            .map(str::to_string);

        if remote_dsn.is_none() {
            prune_historical_rows_with_retention(&local, &repo.repo_id, PRE_PUSH_RETENTION_COMMITS)
                .await
                .context("pruning local historical DevQL rows with retention policy")?;
            return Ok(());
        }

        let updates = collect_pre_push_ref_updates(&stdin_lines);
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
                    let _ = mark_branch_sync_pending(
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
                    let _ = mark_branch_sync_pending(
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

        let relational =
            crate::host::devql::RelationalStorage::with_remote_client(sqlite_path, client);
        let mut synced_heads: Vec<String> = Vec::new();
        for update in &updates {
            mark_branch_sync_pending(
                &local,
                &repo.repo_id,
                remote_name,
                &update.remote_branch,
                &update.local_sha,
            )
            .await?;

            let commits_to_sync = list_commits_to_sync_for_ref_update(
                &repo_root,
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
                replicate_history_for_commit(&relational, &repo.repo_id, &commit_sha)
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
            sync_remote_branch_current_state(
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

            mark_branch_sync_complete(
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
            prune_historical_rows_up_to_commit(&local, &repo.repo_id, &commit_sha)
                .await
                .with_context(|| {
                    format!("pruning local historical DevQL rows after syncing commit {commit_sha}")
                })?;
        }

        Ok::<(), anyhow::Error>(())
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(sync_future));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime for pre-push DevQL sync")?;
    runtime.block_on(sync_future)
}

fn collect_pre_push_ref_updates(stdin_lines: &[String]) -> Vec<PrePushRefUpdate> {
    let mut updates = Vec::new();
    for line in stdin_lines {
        let Some(update) = parse_pre_push_update_line(line) else {
            continue;
        };
        if is_zero_git_oid(&update.local_sha) {
            continue;
        }
        updates.push(update);
    }
    updates
}

fn parse_pre_push_update_line(line: &str) -> Option<PrePushRefUpdate> {
    let mut parts = line.split_whitespace();
    let local_ref = parts.next()?.trim().to_string();
    let local_sha = parts.next()?.trim().to_string();
    let remote_ref = parts.next()?.trim().to_string();
    let remote_sha = parts.next()?.trim().to_string();
    if parts.next().is_some() {
        return None;
    }

    if local_ref.is_empty()
        || local_sha.is_empty()
        || remote_ref.is_empty()
        || remote_sha.is_empty()
    {
        return None;
    }

    if local_ref == "(delete)" {
        return None;
    }

    let remote_branch = remote_ref.strip_prefix("refs/heads/")?.trim().to_string();
    if remote_branch.is_empty() {
        return None;
    }

    let local_branch = local_ref
        .strip_prefix("refs/heads/")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Some(PrePushRefUpdate {
        local_ref,
        local_sha,
        remote_ref,
        remote_sha,
        local_branch,
        remote_branch,
    })
}

async fn list_commits_to_sync_for_ref_update(
    repo_root: &Path,
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    update: &PrePushRefUpdate,
) -> Result<Vec<String>> {
    let watermark_key = branch_sync_watermark_key(remote_name, &update.remote_branch);
    let last_synced = load_sync_state_value(local, repo_id, &watermark_key).await?;

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

    if is_zero_git_oid(&update.remote_sha) {
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
        let mut commits = parse_sha_lines(&candidate);
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
    Ok(parse_sha_lines(&output))
}

fn parse_sha_lines(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let sha = line.trim();
        if sha.is_empty() {
            continue;
        }
        if out.iter().any(|existing| existing == sha) {
            continue;
        }
        out.push(sha.to_string());
    }
    out
}

fn git_is_ancestor(repo_root: &Path, ancestor: &str, descendant: &str) -> bool {
    run_git(
        repo_root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )
    .is_ok()
}

async fn replicate_history_for_commit(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
) -> Result<()> {
    let commit_rows = relational
        .query_rows(&format!(
            "SELECT commit_sha, author_name, author_email, commit_message, committed_at \
FROM commits WHERE repo_id = '{}' AND commit_sha = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let file_state_rows = relational
        .query_rows(&format!(
            "SELECT commit_sha, path, blob_sha FROM file_state \
WHERE repo_id = '{}' AND commit_sha = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let artefact_rows = relational
        .query_rows(&format!(
            "SELECT artefact_id, symbol_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts WHERE repo_id = '{}' AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}'\
)",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let edge_rows = relational
        .query_rows(&format!(
            "SELECT edge_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
FROM artefact_edges WHERE repo_id = '{}' AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}'\
)",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;

    let mut statements = Vec::new();
    statements.push(build_commit_replication_sql(
        repo_id,
        commit_sha,
        commit_rows.first(),
    ));
    statements.extend(build_file_state_replication_sql(repo_id, &file_state_rows));
    statements.extend(build_artefacts_replication_sql(repo_id, &artefact_rows));
    statements.extend(build_artefact_edges_replication_sql(repo_id, &edge_rows));

    relational
        .exec_remote_batch_transactional(&statements)
        .await
}

fn build_commit_replication_sql(
    repo_id: &str,
    commit_sha: &str,
    commit_row: Option<&serde_json::Value>,
) -> String {
    let author_name = sql_nullable_text(commit_row.and_then(|row| row_text(row, "author_name")));
    let author_email = sql_nullable_text(commit_row.and_then(|row| row_text(row, "author_email")));
    let commit_message =
        sql_nullable_text(commit_row.and_then(|row| row_text(row, "commit_message")));
    let committed_at =
        sql_nullable_timestamptz(commit_row.and_then(|row| row_text(row, "committed_at")));

    format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) \
VALUES ('{}', '{}', {}, {}, {}, {}) \
ON CONFLICT (commit_sha) DO UPDATE SET \
repo_id = EXCLUDED.repo_id, \
author_name = EXCLUDED.author_name, \
author_email = EXCLUDED.author_email, \
commit_message = EXCLUDED.commit_message, \
committed_at = EXCLUDED.committed_at",
        crate::host::devql::esc_pg(commit_sha),
        crate::host::devql::esc_pg(repo_id),
        author_name,
        author_email,
        commit_message,
        committed_at,
    )
}

fn build_file_state_replication_sql(repo_id: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(commit_sha) = row_text(row, "commit_sha") else {
                continue;
            };
            let Some(path) = row_text(row, "path") else {
                continue;
            };
            let Some(blob_sha) = row_text(row, "blob_sha") else {
                continue;
            };
            values.push(format!(
                "('{}', '{}', '{}', '{}')",
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(blob_sha),
            ));
        }
        if values.is_empty() {
            continue;
        }
        statements.push(format!(
            "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) VALUES {} \
ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha",
            values.join(","),
        ));
    }
    statements
}

fn build_artefacts_replication_sql(repo_id: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(artefact_id) = row_text(row, "artefact_id") else {
                continue;
            };
            let Some(blob_sha) = row_text(row, "blob_sha") else {
                continue;
            };
            let Some(path) = row_text(row, "path") else {
                continue;
            };
            let Some(language) = row_text(row, "language") else {
                continue;
            };

            let symbol_id = sql_nullable_text(row_text(row, "symbol_id"));
            let canonical_kind = sql_nullable_text(row_text(row, "canonical_kind"));
            let language_kind = sql_nullable_text(row_text(row, "language_kind"));
            let symbol_fqn = sql_nullable_text(row_text(row, "symbol_fqn"));
            let parent_artefact_id = sql_nullable_text(row_text(row, "parent_artefact_id"));
            let start_line = row_i64(row, "start_line").unwrap_or_default();
            let end_line = row_i64(row, "end_line").unwrap_or_default();
            let start_byte = row_i64(row, "start_byte").unwrap_or_default();
            let end_byte = row_i64(row, "end_byte").unwrap_or_default();
            let signature = sql_nullable_text(row_text(row, "signature"));
            let modifiers = sql_jsonb_text(row.get("modifiers"), "[]");
            let docstring = sql_nullable_text(row_text(row, "docstring"));
            let content_hash = sql_nullable_text(row_text(row, "content_hash"));

            values.push(format!(
                "('{}', {}, '{}', '{}', '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::host::devql::esc_pg(artefact_id),
                symbol_id,
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(language),
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_artefact_id,
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature,
                modifiers,
                docstring,
                content_hash,
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash) VALUES {} \
ON CONFLICT (artefact_id) DO NOTHING",
            values.join(","),
        ));
    }
    statements
}

fn build_artefact_edges_replication_sql(repo_id: &str, rows: &[serde_json::Value]) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(edge_id) = row_text(row, "edge_id") else {
                continue;
            };
            let Some(blob_sha) = row_text(row, "blob_sha") else {
                continue;
            };
            let Some(from_artefact_id) = row_text(row, "from_artefact_id") else {
                continue;
            };
            let Some(edge_kind) = row_text(row, "edge_kind") else {
                continue;
            };
            let Some(language) = row_text(row, "language") else {
                continue;
            };

            let to_artefact_id = row_text(row, "to_artefact_id");
            let to_symbol_ref = row_text(row, "to_symbol_ref");
            if to_artefact_id.is_none() && to_symbol_ref.is_none() {
                continue;
            }

            let start_line = sql_nullable_i64(row_i64(row, "start_line"));
            let end_line = sql_nullable_i64(row_i64(row, "end_line"));
            let metadata = sql_jsonb_text(row.get("metadata"), "{}");

            values.push(format!(
                "('{}', '{}', '{}', '{}', {}, {}, '{}', '{}', {}, {}, {})",
                crate::host::devql::esc_pg(edge_id),
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(from_artefact_id),
                sql_nullable_text(to_artefact_id.as_deref()),
                sql_nullable_text(to_symbol_ref.as_deref()),
                crate::host::devql::esc_pg(edge_kind),
                crate::host::devql::esc_pg(language),
                start_line,
                end_line,
                metadata,
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefact_edges (edge_id, repo_id, blob_sha, from_artefact_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata) VALUES {} \
ON CONFLICT (edge_id) DO NOTHING",
            values.join(","),
        ));
    }
    statements
}

async fn sync_remote_branch_current_state(
    relational: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    source_branch: &str,
    remote_branch: &str,
) -> Result<()> {
    let artefact_rows = relational
        .query_rows(&format!(
            "SELECT symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash \
FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}' AND revision_kind = 'commit'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(source_branch),
        ))
        .await?;
    let edge_rows = relational
        .query_rows(&format!(
            "SELECT edge_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata \
FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}' AND revision_kind = 'commit'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(source_branch),
        ))
        .await?;

    let mut statements = vec![
        format!(
            "DELETE FROM artefact_edges_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(remote_branch),
        ),
        format!(
            "DELETE FROM artefacts_current WHERE repo_id = '{}' AND branch = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(remote_branch),
        ),
    ];
    statements.extend(build_artefacts_current_replication_sql(
        repo_id,
        remote_branch,
        &artefact_rows,
    ));
    statements.extend(build_artefact_edges_current_replication_sql(
        repo_id,
        remote_branch,
        &edge_rows,
    ));
    relational
        .exec_remote_batch_transactional(&statements)
        .await
}

fn build_artefacts_current_replication_sql(
    repo_id: &str,
    remote_branch: &str,
    rows: &[serde_json::Value],
) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(symbol_id) = row_text(row, "symbol_id") else {
                continue;
            };
            let Some(artefact_id) = row_text(row, "artefact_id") else {
                continue;
            };
            let Some(commit_sha) = row_text(row, "commit_sha") else {
                continue;
            };
            let Some(blob_sha) = row_text(row, "blob_sha") else {
                continue;
            };
            let Some(path) = row_text(row, "path") else {
                continue;
            };
            let Some(language) = row_text(row, "language") else {
                continue;
            };

            let revision_kind = row_text(row, "revision_kind").unwrap_or("commit");
            let revision_id = row_text(row, "revision_id").unwrap_or("");
            let temp_checkpoint_id = sql_nullable_i64(row_i64(row, "temp_checkpoint_id"));
            let canonical_kind = sql_nullable_text(row_text(row, "canonical_kind"));
            let language_kind = sql_nullable_text(row_text(row, "language_kind"));
            let symbol_fqn = sql_nullable_text(row_text(row, "symbol_fqn"));
            let parent_symbol_id = sql_nullable_text(row_text(row, "parent_symbol_id"));
            let parent_artefact_id = sql_nullable_text(row_text(row, "parent_artefact_id"));
            let start_line = row_i64(row, "start_line").unwrap_or_default();
            let end_line = row_i64(row, "end_line").unwrap_or_default();
            let start_byte = row_i64(row, "start_byte").unwrap_or_default();
            let end_byte = row_i64(row, "end_byte").unwrap_or_default();
            let signature = sql_nullable_text(row_text(row, "signature"));
            let modifiers = sql_jsonb_text(row.get("modifiers"), "[]");
            let docstring = sql_nullable_text(row_text(row, "docstring"));
            let content_hash = sql_nullable_text(row_text(row, "content_hash"));

            values.push(format!(
                "('{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(remote_branch),
                crate::host::devql::esc_pg(symbol_id),
                crate::host::devql::esc_pg(artefact_id),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(revision_kind),
                crate::host::devql::esc_pg(revision_id),
                temp_checkpoint_id,
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(language),
                canonical_kind,
                language_kind,
                symbol_fqn,
                parent_symbol_id,
                parent_artefact_id,
                start_line,
                end_line,
                start_byte,
                end_byte,
                signature,
                modifiers,
                docstring,
                content_hash,
                "now()",
            ));
        }
        if values.is_empty() {
            continue;
        }
        statements.push(format!(
            "INSERT INTO artefacts_current (repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at) VALUES {}",
            values.join(","),
        ));
    }
    statements
}

fn build_artefact_edges_current_replication_sql(
    repo_id: &str,
    remote_branch: &str,
    rows: &[serde_json::Value],
) -> Vec<String> {
    let mut statements = Vec::new();
    for chunk in rows.chunks(PRE_PUSH_BATCH_SIZE) {
        let mut values = Vec::new();
        for row in chunk {
            let Some(edge_id) = row_text(row, "edge_id") else {
                continue;
            };
            let Some(commit_sha) = row_text(row, "commit_sha") else {
                continue;
            };
            let Some(blob_sha) = row_text(row, "blob_sha") else {
                continue;
            };
            let Some(path) = row_text(row, "path") else {
                continue;
            };
            let Some(from_symbol_id) = row_text(row, "from_symbol_id") else {
                continue;
            };
            let Some(from_artefact_id) = row_text(row, "from_artefact_id") else {
                continue;
            };
            let Some(edge_kind) = row_text(row, "edge_kind") else {
                continue;
            };
            let Some(language) = row_text(row, "language") else {
                continue;
            };

            let to_symbol_id = row_text(row, "to_symbol_id");
            let to_symbol_ref = row_text(row, "to_symbol_ref");
            if to_symbol_id.is_none() && to_symbol_ref.is_none() {
                continue;
            }

            let revision_kind = row_text(row, "revision_kind").unwrap_or("commit");
            let revision_id = row_text(row, "revision_id").unwrap_or("");
            let temp_checkpoint_id = sql_nullable_i64(row_i64(row, "temp_checkpoint_id"));
            let to_artefact_id = sql_nullable_text(row_text(row, "to_artefact_id"));
            let start_line = sql_nullable_i64(row_i64(row, "start_line"));
            let end_line = sql_nullable_i64(row_i64(row, "end_line"));
            let metadata = sql_jsonb_text(row.get("metadata"), "{}");

            values.push(format!(
                "('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}', '{}', {}, {}, {}, '{}', '{}', {}, {}, {}, {})",
                crate::host::devql::esc_pg(edge_id),
                crate::host::devql::esc_pg(repo_id),
                crate::host::devql::esc_pg(remote_branch),
                crate::host::devql::esc_pg(commit_sha),
                crate::host::devql::esc_pg(revision_kind),
                crate::host::devql::esc_pg(revision_id),
                temp_checkpoint_id,
                crate::host::devql::esc_pg(blob_sha),
                crate::host::devql::esc_pg(path),
                crate::host::devql::esc_pg(from_symbol_id),
                crate::host::devql::esc_pg(from_artefact_id),
                sql_nullable_text(to_symbol_id.as_deref()),
                to_artefact_id,
                sql_nullable_text(to_symbol_ref.as_deref()),
                crate::host::devql::esc_pg(edge_kind),
                crate::host::devql::esc_pg(language),
                start_line,
                end_line,
                metadata,
                "now()",
            ));
        }
        if values.is_empty() {
            continue;
        }

        statements.push(format!(
            "INSERT INTO artefact_edges_current (edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) VALUES {}",
            values.join(","),
        ));
    }
    statements
}

async fn mark_branch_sync_pending(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    remote_branch: &str,
    local_sha: &str,
) -> Result<()> {
    let key = branch_sync_pending_key(remote_name, remote_branch);
    let sql = build_sync_state_upsert_sql(repo_id, &key, local_sha);
    local.exec(&sql).await
}

async fn mark_branch_sync_complete(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    remote_name: &str,
    remote_branch: &str,
    local_sha: &str,
) -> Result<()> {
    let watermark_key = branch_sync_watermark_key(remote_name, remote_branch);
    let pending_key = branch_sync_pending_key(remote_name, remote_branch);
    let statements = vec![
        build_sync_state_upsert_sql(repo_id, PRE_PUSH_SYNC_WATERMARK_KEY, local_sha),
        build_sync_state_upsert_sql(repo_id, &watermark_key, local_sha),
        format!(
            "DELETE FROM sync_state WHERE repo_id = '{}' AND state_key = '{}'",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(&pending_key),
        ),
    ];
    local.exec_batch_transactional(&statements).await
}

fn branch_sync_watermark_key(remote_name: &str, remote_branch: &str) -> String {
    format!(
        "{}:{}:{}",
        PRE_PUSH_SYNC_WATERMARK_KEY,
        remote_name.trim(),
        remote_branch.trim()
    )
}

fn branch_sync_pending_key(remote_name: &str, remote_branch: &str) -> String {
    format!(
        "{}:{}:{}",
        PRE_PUSH_SYNC_PENDING_KEY_PREFIX,
        remote_name.trim(),
        remote_branch.trim()
    )
}

async fn load_sync_state_value(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    state_key: &str,
) -> Result<Option<String>> {
    let rows = local
        .query_rows(&format!(
            "SELECT state_value FROM sync_state \
WHERE repo_id = '{}' AND state_key = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(state_key),
        ))
        .await?;
    Ok(rows
        .first()
        .and_then(|row| row_text(row, "state_value"))
        .map(str::to_string))
}

fn build_sync_state_upsert_sql(repo_id: &str, state_key: &str, state_value: &str) -> String {
    format!(
        "INSERT INTO sync_state (repo_id, state_key, state_value, updated_at) VALUES ('{}', '{}', '{}', datetime('now')) \
ON CONFLICT (repo_id, state_key) DO UPDATE SET state_value = EXCLUDED.state_value, updated_at = datetime('now')",
        crate::host::devql::esc_pg(repo_id),
        crate::host::devql::esc_pg(state_key),
        crate::host::devql::esc_pg(state_value),
    )
}

async fn prune_historical_rows_up_to_commit(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
) -> Result<()> {
    let commit_rows = local
        .query_rows(&format!(
            "SELECT committed_at FROM commits WHERE repo_id = '{}' AND commit_sha = '{}' LIMIT 1",
            crate::host::devql::esc_pg(repo_id),
            crate::host::devql::esc_pg(commit_sha),
        ))
        .await?;
    let Some(committed_at) = commit_rows
        .first()
        .and_then(|row| row_text(row, "committed_at"))
    else {
        return Ok(());
    };
    if committed_at.trim().is_empty() {
        return Ok(());
    }

    let statements = build_prune_before_timestamp_sql(repo_id, committed_at);
    local.exec_batch_transactional(&statements).await
}

async fn prune_historical_rows_with_retention(
    local: &crate::host::devql::RelationalStorage,
    repo_id: &str,
    keep_commits: usize,
) -> Result<()> {
    let statements = build_retention_prune_sql(repo_id, keep_commits);
    local.exec_batch_transactional(&statements).await
}

fn build_prune_before_timestamp_sql(repo_id: &str, committed_at: &str) -> Vec<String> {
    let repo_id = crate::host::devql::esc_pg(repo_id);
    let committed_at = crate::host::devql::esc_pg(committed_at);
    vec![
        format!(
            "DELETE FROM artefacts \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha \
    FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha IN (\
        SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
      )\
  )"
        ),
        format!(
            "DELETE FROM artefact_edges \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha \
    FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha IN (\
        SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
      )\
  )"
        ),
        format!(
            "DELETE FROM file_state \
WHERE repo_id = '{repo_id}' \
  AND commit_sha IN (\
    SELECT commit_sha FROM commits WHERE repo_id = '{repo_id}' AND committed_at <= '{committed_at}'\
  )"
        ),
    ]
}

fn build_retention_prune_sql(repo_id: &str, keep_commits: usize) -> Vec<String> {
    let repo_id = crate::host::devql::esc_pg(repo_id);
    vec![
        format!(
            "DELETE FROM artefacts \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha NOT IN (\
        SELECT commit_sha \
        FROM commits \
        WHERE repo_id = '{repo_id}' \
        ORDER BY committed_at DESC, commit_sha DESC \
        LIMIT {keep_commits}\
      )\
  )"
        ),
        format!(
            "DELETE FROM artefact_edges \
WHERE repo_id = '{repo_id}' \
  AND blob_sha IN (\
    SELECT DISTINCT blob_sha FROM file_state \
    WHERE repo_id = '{repo_id}' \
      AND commit_sha NOT IN (\
        SELECT commit_sha \
        FROM commits \
        WHERE repo_id = '{repo_id}' \
        ORDER BY committed_at DESC, commit_sha DESC \
        LIMIT {keep_commits}\
      )\
  )"
        ),
        format!(
            "DELETE FROM file_state \
WHERE repo_id = '{repo_id}' \
  AND commit_sha NOT IN (\
    SELECT commit_sha \
    FROM commits \
    WHERE repo_id = '{repo_id}' \
    ORDER BY committed_at DESC, commit_sha DESC \
    LIMIT {keep_commits}\
  )"
        ),
    ]
}

fn row_text<'a>(row: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn row_i64(row: &serde_json::Value, key: &str) -> Option<i64> {
    row.get(key).and_then(|value| match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    })
}

fn sql_nullable_text(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!("'{}'", crate::host::devql::esc_pg(value)),
        None => "NULL".to_string(),
    }
}

fn sql_nullable_i64(value: Option<i64>) -> String {
    value
        .map(|number| number.to_string())
        .unwrap_or_else(|| "NULL".to_string())
}

fn sql_nullable_timestamptz(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => format!(
            "NULLIF('{}', '')::timestamptz",
            crate::host::devql::esc_pg(value)
        ),
        None => "NULL".to_string(),
    }
}

fn sql_jsonb_text(value: Option<&serde_json::Value>, default_json: &str) -> String {
    let json_text = match value {
        Some(serde_json::Value::Null) | None => default_json.to_string(),
        Some(serde_json::Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                default_json.to_string()
            } else if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
                trimmed.to_string()
            } else {
                default_json.to_string()
            }
        }
        Some(other) => other.to_string(),
    };
    format!("'{}'::jsonb", crate::host::devql::esc_pg(&json_text))
}

#[cfg(test)]
mod pre_push_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_pre_push_update_line_accepts_branch_refs() {
        let line = "refs/heads/main abc123 refs/heads/main def456";
        let parsed = parse_pre_push_update_line(line).expect("parse branch update");
        assert_eq!(parsed.local_ref, "refs/heads/main");
        assert_eq!(parsed.remote_ref, "refs/heads/main");
        assert_eq!(parsed.local_branch.as_deref(), Some("main"));
        assert_eq!(parsed.remote_branch, "main");
    }

    #[test]
    fn parse_pre_push_update_line_rejects_non_branch_remote_ref() {
        let line = "refs/heads/main abc123 refs/tags/v1 def456";
        assert!(
            parse_pre_push_update_line(line).is_none(),
            "tag pushes should be ignored by pre-push replication"
        );
    }

    #[test]
    fn build_artefacts_replication_sql_targets_expected_columns() {
        let rows = vec![json!({
            "artefact_id": "a1",
            "symbol_id": "s1",
            "blob_sha": "b1",
            "path": "src/lib.rs",
            "language": "rust",
            "canonical_kind": "function",
            "language_kind": "function_item",
            "symbol_fqn": "src/lib.rs::run",
            "parent_artefact_id": null,
            "start_line": 1,
            "end_line": 3,
            "start_byte": 0,
            "end_byte": 10,
            "signature": "fn run()",
            "modifiers": "[]",
            "docstring": "test",
            "content_hash": "hash-1"
        })];

        let sql = build_artefacts_replication_sql("repo-1", &rows).join("\n");
        assert!(sql.contains("INSERT INTO artefacts"));
        assert!(sql.contains("content_hash"));
        assert!(
            !sql.contains("created_at, created_at"),
            "artefacts replication SQL must not duplicate created_at columns"
        );
    }
}

fn is_zero_git_oid(value: &str) -> bool {
    value.trim() == ZERO_GIT_OID
}
