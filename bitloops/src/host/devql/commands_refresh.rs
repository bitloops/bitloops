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
    commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for post-commit artefact refresh")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "git post-commit artefact refresh",
    )
    .await?;

    update_artefacts_for_changed_files(cfg, &relational, commit_sha, changed_files, "post-commit")
        .await
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

    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
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
    commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for post-merge artefact refresh")?;
    let relational =
        RelationalStorage::connect(cfg, &backends.relational, "git post-merge artefact refresh")
            .await?;

    update_artefacts_for_changed_files(cfg, &relational, commit_sha, changed_files, "post-merge")
        .await
}

async fn update_artefacts_for_changed_files(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_sha: &str,
    changed_files: &[String],
    source_hook: &str,
) -> Result<PostCommitArtefactRefreshStats> {
    ensure_repository_row(cfg, relational).await?;

    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, commit_sha).unwrap_or(
        CheckpointCommitInfo {
            commit_sha: commit_sha.to_string(),
            commit_unix: 0,
            author_name: String::new(),
            author_email: String::new(),
            subject: String::new(),
        },
    );
    upsert_commit_metadata_row(cfg, relational, &commit_info).await?;

    let mut stats = PostCommitArtefactRefreshStats::default();
    for raw_path in changed_files {
        let path = normalize_repo_path(raw_path);
        if path.is_empty() {
            continue;
        }
        stats.files_seen += 1;

        let refresh_result = async {
            let blob_sha = git_blob_sha_at_commit(&cfg.repo_root, commit_sha, &path)
                .or_else(|| git_blob_sha_at_commit(&cfg.repo_root, commit_sha, raw_path));
            if let Some(blob_sha) = blob_sha {
                upsert_file_state_row(&cfg.repo.repo_id, relational, commit_sha, &path, &blob_sha)
                    .await?;
                let file_artefact = upsert_file_artefact_row(
                    &cfg.repo.repo_id,
                    &cfg.repo_root,
                    relational,
                    &path,
                    &blob_sha,
                )
                .await?;
                upsert_language_artefacts(
                    cfg,
                    relational,
                    &FileRevision {
                        commit_sha,
                        revision: TemporalRevisionRef {
                            kind: TemporalRevisionKind::Commit,
                            id: commit_sha,
                            temp_checkpoint_id: None,
                        },
                        commit_unix: commit_info.commit_unix,
                        path: &path,
                        blob_sha: &blob_sha,
                    },
                    &file_artefact,
                )
                .await?;
                stats.files_indexed += 1;
            } else {
                delete_current_state_for_path(cfg, relational, &path).await?;
                stats.files_deleted += 1;
            }
            Result::<(), anyhow::Error>::Ok(())
        }
        .await;

        if let Err(err) = refresh_result {
            stats.files_failed += 1;
            eprintln!(
                "[bitloops] Warning: DevQL {source_hook} refresh failed for `{path}` at commit {commit_sha}: {err:#}"
            );
        }
    }

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
    .ok_or_else(|| {
        anyhow::anyhow!("checkpoint not found for projection refresh: {checkpoint_id}")
    })?;
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

pub async fn run_post_checkout_branch_seed(
    cfg: &DevqlConfig,
    previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout || new_head.trim().is_empty() || is_zero_git_oid(new_head) {
        return Ok(());
    }

    let new_branch = run_git(&cfg.repo_root, &["branch", "--show-current"])
        .ok()
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    if new_branch.is_empty() {
        return Ok(());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for post-checkout branch seeding")?;
    let relational =
        RelationalStorage::connect(cfg, &backends.relational, "git post-checkout branch seed")
            .await?;

    ensure_repository_row(cfg, &relational).await?;
    if branch_has_current_state_rows(cfg, &relational, &new_branch).await? {
        return Ok(());
    }

    if previous_head.trim() == new_head.trim()
        && !previous_head.trim().is_empty()
        && !is_zero_git_oid(previous_head)
        && let Some(source_branch) =
            resolve_fast_copy_source_branch(cfg, &relational, previous_head, &new_branch).await?
        && copy_branch_current_state(cfg, &relational, &source_branch, &new_branch).await?
    {
        return Ok(());
    }

    let files = discover_baseline_files_at_revision(&cfg.repo_root, new_head)?;
    let _stats =
        update_artefacts_for_changed_files(cfg, &relational, new_head, &files, "post-checkout")
            .await?;
    Ok(())
}

fn is_zero_git_oid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '0')
}

async fn branch_has_current_state_rows(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    branch: &str,
) -> Result<bool> {
    let sql = format!(
        "SELECT COUNT(*) AS row_count FROM artefacts_current \
WHERE repo_id = '{}' AND branch = '{}'",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(branch),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(count_query_rows(&rows) > 0)
}

fn count_query_rows(rows: &[Value]) -> usize {
    rows.first()
        .and_then(|row| row.get("row_count"))
        .and_then(|value| {
            value
                .as_u64()
                .map(|number| number as usize)
                .or_else(|| value.as_i64().map(|number| number.max(0) as usize))
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<usize>().ok()))
        })
        .unwrap_or_default()
}

async fn resolve_fast_copy_source_branch(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    previous_head: &str,
    new_branch: &str,
) -> Result<Option<String>> {
    let refs = run_git(
        &cfg.repo_root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "--points-at",
            previous_head,
            "refs/heads",
        ],
    )
    .unwrap_or_default();
    let mut candidates = refs
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != new_branch)
        .map(str::to_string)
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();

    for branch in candidates {
        if branch_has_current_state_rows(cfg, relational, &branch).await? {
            return Ok(Some(branch));
        }
    }

    Ok(None)
}

async fn copy_branch_current_state(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    source_branch: &str,
    target_branch: &str,
) -> Result<bool> {
    if !branch_has_current_state_rows(cfg, relational, source_branch).await? {
        return Ok(false);
    }

    let copy_artefacts_sql = format!(
        "INSERT INTO artefacts_current (repo_id, branch, symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at) \
SELECT repo_id, '{}', symbol_id, artefact_id, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, language, canonical_kind, language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature, modifiers, docstring, content_hash, updated_at \
FROM artefacts_current \
WHERE repo_id = '{}' AND branch = '{}' \
ON CONFLICT (repo_id, branch, symbol_id) DO NOTHING",
        esc_pg(target_branch),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(source_branch),
    );
    let copy_edges_sql = format!(
        "INSERT INTO artefact_edges_current (edge_id, repo_id, branch, commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at) \
SELECT edge_id, repo_id, '{}', commit_sha, revision_kind, revision_id, temp_checkpoint_id, blob_sha, path, from_symbol_id, from_artefact_id, to_symbol_id, to_artefact_id, to_symbol_ref, edge_kind, language, start_line, end_line, metadata, updated_at \
FROM artefact_edges_current \
WHERE repo_id = '{}' AND branch = '{}' \
ON CONFLICT (repo_id, branch, edge_id) DO NOTHING",
        esc_pg(target_branch),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(source_branch),
    );
    relational
        .exec_batch_transactional(&[copy_artefacts_sql, copy_edges_sql])
        .await?;

    Ok(true)
}
