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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectionRowKey {
    path: String,
    blob_sha: String,
}

#[derive(Debug, Clone)]
struct ProjectionRow {
    repo_id: String,
    checkpoint_id: String,
    session_id: String,
    event_time: String,
    agent: String,
    branch: String,
    strategy: String,
    commit_sha: String,
    path: String,
    blob_sha: String,
}

impl ProjectionRow {
    fn key(&self) -> ProjectionRowKey {
        ProjectionRowKey {
            path: self.path.clone(),
            blob_sha: self.blob_sha.clone(),
        }
    }
}

pub async fn run_checkpoint_file_snapshot_backfill(
    cfg: &DevqlConfig,
    mut options: CheckpointFileSnapshotBackfillOptions,
) -> Result<()> {
    options.resume_after = normalise_optional_resume_after(options.resume_after);
    validate_backfill_options(&options)?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root).context(
        "resolving DevQL backend config for `devql projection checkpoint-file-snapshots`",
    )?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "devql projection checkpoint-file-snapshots",
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
        crate::host::checkpoints::strategy::manual_commit::resolve_temporary_checkpoint_sqlite_path(
            &cfg.repo_root,
        )?;
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
        let existing_keys =
            load_existing_projection_keys(relational, &cfg.repo.repo_id, &checkpoint.checkpoint_id)
                .await?;
        let (resolved_rows, unresolved_files) = resolve_projection_rows_for_checkpoint(
            cfg,
            relational,
            &checkpoint,
            &commit_info.commit_sha,
            &event_time,
        )
        .await?;

        let resolved_keys = resolved_rows
            .iter()
            .map(ProjectionRow::key)
            .collect::<HashSet<_>>();
        let stale_keys = existing_keys
            .difference(&resolved_keys)
            .cloned()
            .collect::<Vec<_>>();
        let missing_rows = resolved_rows
            .into_iter()
            .filter(|row| !existing_keys.contains(&row.key()))
            .collect::<Vec<_>>();

        summary.checkpoints_processed += 1;
        summary.rows_projected += missing_rows.len();
        summary.rows_already_present += resolved_keys.len().saturating_sub(missing_rows.len());
        summary.stale_rows_detected += stale_keys.len();
        summary.unresolved_files += unresolved_files;
        checkpoints_in_batch += 1;

        if !options.dry_run {
            for row in &missing_rows {
                sqlite_statements.push(insert_projection_row_sql(row, RelationalDialect::Sqlite));
                if relational.remote.is_some() {
                    postgres_statements
                        .push(insert_projection_row_sql(row, RelationalDialect::Postgres));
                }
            }

            if unresolved_files == 0 {
                summary.stale_rows_deleted += stale_keys.len();
                for key in stale_keys {
                    sqlite_statements.push(delete_projection_row_sql(
                        &cfg.repo.repo_id,
                        &checkpoint.checkpoint_id,
                        &key,
                    ));
                    if relational.remote.is_some() {
                        postgres_statements.push(delete_projection_row_sql(
                            &cfg.repo.repo_id,
                            &checkpoint.checkpoint_id,
                            &key,
                        ));
                    }
                }
            }
        }

        if checkpoints_in_batch >= options.batch_size {
            flush_projection_batch(relational, &mut sqlite_statements, &mut postgres_statements)
                .await?;
            checkpoints_in_batch = 0;
            if options.emit_progress {
                println!(
                    "checkpoint_file_snapshots progress: checkpoints_processed={}, rows_projected={}, rows_already_present={}, stale_rows_deleted={}, stale_rows_detected={}, unresolved_files={}, last_checkpoint_id={}",
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

async fn resolve_projection_rows_for_checkpoint(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    checkpoint: &CommittedInfo,
    commit_sha: &str,
    event_time: &str,
) -> Result<(Vec<ProjectionRow>, usize)> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mut unresolved_files = 0usize;

    for raw_path in &checkpoint.files_touched {
        let normalized_path = normalize_repo_path(raw_path);
        if normalized_path.is_empty() {
            continue;
        }

        let Some(blob_sha) = load_file_state_blob_sha(
            relational,
            &cfg.repo.repo_id,
            commit_sha,
            &normalized_path,
            raw_path,
        )
        .await?
        else {
            unresolved_files += 1;
            continue;
        };

        let row = ProjectionRow {
            repo_id: cfg.repo.repo_id.clone(),
            checkpoint_id: checkpoint.checkpoint_id.clone(),
            session_id: checkpoint.session_id.clone(),
            event_time: event_time.to_string(),
            agent: checkpoint.agent.clone(),
            branch: checkpoint.branch.clone(),
            strategy: checkpoint.strategy.clone(),
            commit_sha: commit_sha.to_string(),
            path: normalized_path,
            blob_sha,
        };

        if seen.insert(row.key()) {
            rows.push(row);
        }
    }

    Ok((rows, unresolved_files))
}

async fn load_file_state_blob_sha(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    normalized_path: &str,
    raw_path: &str,
) -> Result<Option<String>> {
    let path_predicate = if normalized_path == raw_path {
        format!("path = '{}'", esc_pg(normalized_path))
    } else {
        format!(
            "(path = '{}' OR path = '{}')",
            esc_pg(normalized_path),
            esc_pg(raw_path),
        )
    };
    let sql = format!(
        "SELECT blob_sha, path FROM file_state \
WHERE repo_id = '{}' AND commit_sha = '{}' AND {} \
ORDER BY CASE WHEN path = '{}' THEN 0 ELSE 1 END \
LIMIT 1",
        esc_pg(repo_id),
        esc_pg(commit_sha),
        path_predicate,
        esc_pg(normalized_path),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("blob_sha"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

async fn load_existing_projection_keys(
    relational: &RelationalStorage,
    repo_id: &str,
    checkpoint_id: &str,
) -> Result<HashSet<ProjectionRowKey>> {
    let sql = format!(
        "SELECT path, blob_sha FROM checkpoint_file_snapshots \
WHERE repo_id = '{}' AND checkpoint_id = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(ProjectionRowKey {
                path: row.get("path")?.as_str()?.to_string(),
                blob_sha: row.get("blob_sha")?.as_str()?.to_string(),
            })
        })
        .collect())
}

fn insert_projection_row_sql(row: &ProjectionRow, dialect: RelationalDialect) -> String {
    let event_time_sql = projection_event_time_sql(&row.event_time, dialect);
    format!(
        "INSERT INTO checkpoint_file_snapshots (repo_id, checkpoint_id, session_id, event_time, agent, branch, strategy, commit_sha, path, blob_sha) \
VALUES ('{}', '{}', '{}', {}, '{}', '{}', '{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id, checkpoint_id, path, blob_sha) DO UPDATE SET \
session_id = EXCLUDED.session_id, \
event_time = EXCLUDED.event_time, \
agent = EXCLUDED.agent, \
branch = EXCLUDED.branch, \
strategy = EXCLUDED.strategy, \
commit_sha = EXCLUDED.commit_sha",
        esc_pg(&row.repo_id),
        esc_pg(&row.checkpoint_id),
        esc_pg(&row.session_id),
        event_time_sql,
        esc_pg(&row.agent),
        esc_pg(&row.branch),
        esc_pg(&row.strategy),
        esc_pg(&row.commit_sha),
        esc_pg(&row.path),
        esc_pg(&row.blob_sha),
    )
}

fn delete_projection_row_sql(repo_id: &str, checkpoint_id: &str, key: &ProjectionRowKey) -> String {
    format!(
        "DELETE FROM checkpoint_file_snapshots \
WHERE repo_id = '{}' AND checkpoint_id = '{}' AND path = '{}' AND blob_sha = '{}'",
        esc_pg(repo_id),
        esc_pg(checkpoint_id),
        esc_pg(&key.path),
        esc_pg(&key.blob_sha),
    )
}

fn projection_event_time_sql(event_time: &str, dialect: RelationalDialect) -> String {
    let trimmed = event_time.trim();
    match dialect {
        RelationalDialect::Sqlite => format!("'{}'", esc_pg(trimmed)),
        RelationalDialect::Postgres => trimmed
            .parse::<i64>()
            .map(|unix| format!("to_timestamp({unix})"))
            .unwrap_or_else(|_| format!("CAST('{}' AS TIMESTAMPTZ)", esc_pg(trimmed))),
    }
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
