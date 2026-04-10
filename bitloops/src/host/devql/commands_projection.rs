use super::*;

#[derive(Debug, Clone)]
pub struct CheckpointFileSnapshotBackfillOptions {
    pub batch_size: usize,
    pub max_checkpoints: Option<usize>,
    pub resume_after: Option<String>,
    pub dry_run: bool,
    pub emit_progress: bool,
}

impl Default for CheckpointFileSnapshotBackfillOptions {
    fn default() -> Self {
        Self {
            batch_size: 200,
            max_checkpoints: None,
            resume_after: None,
            dry_run: false,
            emit_progress: false,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointFileSnapshotBackfillSummary {
    pub success: bool,
    pub dry_run: bool,
    pub batch_size: usize,
    pub checkpoints_scanned: usize,
    pub checkpoints_processed: usize,
    pub checkpoints_without_commit: usize,
    pub rows_projected: usize,
    pub rows_already_present: usize,
    pub stale_rows_deleted: usize,
    pub stale_rows_detected: usize,
    pub unresolved_files: usize,
    pub resume_after: Option<String>,
    pub last_checkpoint_id: Option<String>,
}

pub async fn run_checkpoint_file_snapshot_backfill(
    cfg: &DevqlConfig,
    mut options: CheckpointFileSnapshotBackfillOptions,
) -> Result<()> {
    options.resume_after = normalise_optional_resume_after(options.resume_after);
    validate_backfill_options(&options)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for `devql projection checkpoint-provenance`")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "devql projection checkpoint-provenance",
    )
    .await?;
    init_relational_schema(cfg, &relational).await?;

    let summary =
        execute_checkpoint_file_snapshot_backfill_with_relational(cfg, &relational, &options)
            .await?;
    println!(
        "{}",
        format_checkpoint_file_snapshot_backfill_summary(&summary)
    );
    Ok(())
}

pub(crate) async fn execute_checkpoint_file_snapshot_backfill_with_relational(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    options: &CheckpointFileSnapshotBackfillOptions,
) -> Result<CheckpointFileSnapshotBackfillSummary> {
    validate_backfill_options(options)?;
    ensure_repository_row(cfg, relational).await?;

    let mut summary = CheckpointFileSnapshotBackfillSummary {
        dry_run: options.dry_run,
        batch_size: options.batch_size,
        resume_after: normalise_optional_resume_after(options.resume_after.clone()),
        ..CheckpointFileSnapshotBackfillSummary::default()
    };

    let checkpoint_db_path =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(
            &cfg.repo_root,
        )?
        .into_inner()
        .local
        .path;
    if !checkpoint_db_path.is_file() {
        summary.success = true;
        return Ok(summary);
    }

    let checkpoints = list_committed(&cfg.repo_root)?;
    let checkpoints = apply_resume_filter(checkpoints, summary.resume_after.as_deref())?;
    let max_checkpoints = options.max_checkpoints.unwrap_or(usize::MAX);
    let checkpoints = checkpoints
        .into_iter()
        .take(max_checkpoints)
        .collect::<Vec<_>>();
    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    let mut sqlite_statements = Vec::new();
    let mut postgres_statements = Vec::new();
    let mut checkpoints_in_batch = 0usize;

    for checkpoint in checkpoints {
        summary.checkpoints_scanned += 1;
        summary.last_checkpoint_id = Some(checkpoint.checkpoint_id.clone());

        let Some(commit_info) = commit_map.get(&checkpoint.checkpoint_id) else {
            summary.checkpoints_without_commit += 1;
            continue;
        };
        if commit_info.commit_sha.trim().is_empty() {
            summary.checkpoints_without_commit += 1;
            continue;
        }

        let event_time = checkpoint_event_time_rfc3339(&checkpoint, Some(commit_info));
        let context = crate::host::devql::checkpoint_provenance::CheckpointProvenanceContext {
            repo_id: &cfg.repo.repo_id,
            checkpoint_id: &checkpoint.checkpoint_id,
            session_id: &checkpoint.session_id,
            event_time: &event_time,
            agent: &checkpoint.agent,
            branch: &checkpoint.branch,
            strategy: &checkpoint.strategy,
            commit_sha: &commit_info.commit_sha,
        };
        let file_rows =
            crate::host::devql::checkpoint_provenance::collect_checkpoint_file_provenance_rows(
                &cfg.repo_root,
                context,
            )?;
        let artefact_provenance =
            crate::host::devql::checkpoint_provenance::collect_checkpoint_artefact_provenance(
                &cfg.repo_root,
                context,
                &file_rows,
            )?;
        let existing_relation_ids = load_existing_checkpoint_file_relation_ids(
            relational,
            &cfg.repo.repo_id,
            &checkpoint.checkpoint_id,
        )
        .await?;
        let new_relation_ids = file_rows
            .iter()
            .map(|row| row.relation_id.clone())
            .collect::<HashSet<_>>();
        let already_present = existing_relation_ids
            .intersection(&new_relation_ids)
            .count();
        let rows_projected = new_relation_ids.len().saturating_sub(already_present);
        let stale_rows = existing_relation_ids.difference(&new_relation_ids).count();
        let unresolved_files = file_rows
            .iter()
            .filter(|row| row.path_after.is_some() && row.blob_sha_after.is_none())
            .count();

        summary.checkpoints_processed += 1;
        summary.rows_projected += rows_projected;
        summary.rows_already_present += already_present;
        summary.stale_rows_detected += stale_rows;
        summary.unresolved_files += unresolved_files;
        checkpoints_in_batch += 1;

        if !options.dry_run {
            sqlite_statements.push(
                crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_lineage_rows_sql(
                    &cfg.repo.repo_id,
                    &checkpoint.checkpoint_id,
                ),
            );
            sqlite_statements.push(
                crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_rows_sql(
                    &cfg.repo.repo_id,
                    &checkpoint.checkpoint_id,
                ),
            );
            sqlite_statements.push(
                crate::host::devql::checkpoint_provenance::delete_checkpoint_file_rows_sql(
                    &cfg.repo.repo_id,
                    &checkpoint.checkpoint_id,
                ),
            );
            if relational.remote.is_some() {
                postgres_statements.push(
                    crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_lineage_rows_sql(
                        &cfg.repo.repo_id,
                        &checkpoint.checkpoint_id,
                    ),
                );
                postgres_statements.push(
                    crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_rows_sql(
                        &cfg.repo.repo_id,
                        &checkpoint.checkpoint_id,
                    ),
                );
                postgres_statements.push(
                    crate::host::devql::checkpoint_provenance::delete_checkpoint_file_rows_sql(
                        &cfg.repo.repo_id,
                        &checkpoint.checkpoint_id,
                    ),
                );
            }

            for row in &file_rows {
                sqlite_statements.push(
                    crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_file_row_sql(
                        row,
                        RelationalDialect::Sqlite,
                    ),
                );
                if relational.remote.is_some() {
                    postgres_statements.push(
                        crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_file_row_sql(
                            row,
                            RelationalDialect::Postgres,
                        ),
                    );
                }
            }
            for row in &artefact_provenance.semantic_rows {
                sqlite_statements.push(
                    crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                        row,
                        RelationalDialect::Sqlite,
                    ),
                );
                if relational.remote.is_some() {
                    postgres_statements.push(
                        crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                            row,
                            RelationalDialect::Postgres,
                        ),
                    );
                }
            }
            for row in &artefact_provenance.lineage_rows {
                sqlite_statements.push(
                    crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_lineage_row_sql(
                        row,
                        RelationalDialect::Sqlite,
                    ),
                );
                if relational.remote.is_some() {
                    postgres_statements.push(
                        crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_lineage_row_sql(
                            row,
                            RelationalDialect::Postgres,
                        ),
                    );
                }
            }
            summary.stale_rows_deleted += stale_rows;
        }

        if checkpoints_in_batch >= options.batch_size {
            flush_projection_batch(relational, &mut sqlite_statements, &mut postgres_statements)
                .await?;
            checkpoints_in_batch = 0;
            if options.emit_progress {
                println!(
                    "checkpoint_provenance progress: checkpoints_processed={}, rows_projected={}, rows_already_present={}, stale_rows_deleted={}, stale_rows_detected={}, unresolved_files={}, last_checkpoint_id={}",
                    summary.checkpoints_processed,
                    summary.rows_projected,
                    summary.rows_already_present,
                    summary.stale_rows_deleted,
                    summary.stale_rows_detected,
                    summary.unresolved_files,
                    summary.last_checkpoint_id.as_deref().unwrap_or("-"),
                );
            }
        }
    }

    flush_projection_batch(relational, &mut sqlite_statements, &mut postgres_statements).await?;
    summary.success = true;
    Ok(summary)
}

fn normalise_optional_resume_after(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn validate_backfill_options(options: &CheckpointFileSnapshotBackfillOptions) -> Result<()> {
    if options.batch_size == 0 {
        bail!("batch_size must be greater than zero");
    }
    if matches!(options.max_checkpoints, Some(0)) {
        bail!("max_checkpoints must be greater than zero when provided");
    }
    Ok(())
}

fn apply_resume_filter(
    checkpoints: Vec<CommittedInfo>,
    resume_after: Option<&str>,
) -> Result<Vec<CommittedInfo>> {
    let Some(resume_after) = resume_after else {
        return Ok(checkpoints);
    };

    let mut filtered = Vec::new();
    let mut found = false;
    for checkpoint in checkpoints {
        if !found {
            if checkpoint.checkpoint_id == resume_after {
                found = true;
            }
            continue;
        }
        filtered.push(checkpoint);
    }

    if !found {
        bail!("checkpoint_id `{resume_after}` was not found in committed checkpoint history");
    }

    Ok(filtered)
}

async fn load_existing_checkpoint_file_relation_ids(
    relational: &RelationalStorage,
    repo_id: &str,
    checkpoint_id: &str,
) -> Result<HashSet<String>> {
    let sql = format!(
        "SELECT relation_id FROM checkpoint_files WHERE repo_id = '{}' AND checkpoint_id = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("relation_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

async fn flush_projection_batch(
    relational: &RelationalStorage,
    sqlite_statements: &mut Vec<String>,
    postgres_statements: &mut Vec<String>,
) -> Result<()> {
    if !sqlite_statements.is_empty() {
        relational
            .exec_batch_transactional(sqlite_statements)
            .await?;
        sqlite_statements.clear();
    }
    if relational.remote.is_some() && !postgres_statements.is_empty() {
        relational
            .exec_remote_batch_transactional(postgres_statements)
            .await?;
        postgres_statements.clear();
    }
    Ok(())
}
