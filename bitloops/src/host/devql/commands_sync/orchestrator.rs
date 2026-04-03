use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::task::JoinSet;

use super::progress::{SyncObserver, SyncProgressPhase, emit_progress};
use super::shared::{
    is_missing_sync_schema_error, load_stored_manifest_for_paths, requested_paths,
    resolve_pack_versions, sync_reason,
};
use super::sqlite_writer::{
    PreparedRemoval, SqliteReadConnectionPool, SqliteSyncWriter, WriterCommitOutcome,
    prepare_sync_item, sync_prepare_worker_count,
};
use super::stats::SyncExecutionStats;
use super::summary::SyncSummary;
use super::validation::execute_sync_validation;
use super::*;

pub async fn run_sync(cfg: &DevqlConfig, mode: sync::types::SyncMode) -> Result<()> {
    run_sync_with_summary(cfg, mode).await.map(|_| ())
}

pub async fn run_sync_with_summary(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
) -> Result<SyncSummary> {
    run_sync_with_summary_and_observer(cfg, mode, None).await
}

pub async fn run_sync_with_summary_and_observer(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<SyncSummary> {
    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
        .context("resolving DevQL backend config for `devql sync`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql sync").await?;
    if matches!(mode, sync::types::SyncMode::Validate) {
        return match execute_sync_validation(cfg, &relational).await {
            Ok(summary) => Ok(summary),
            Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
                "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql sync --validate`.",
            ),
            Err(err) => Err(err),
        };
    }

    match execute_sync_with_observer(cfg, &relational, mode, observer).await {
        Ok(summary) => Ok(summary),
        Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
            "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql sync`.",
        ),
        Err(err) => Err(err),
    }
}

pub(crate) async fn execute_sync(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
) -> Result<SyncSummary> {
    execute_sync_with_observer(cfg, relational, mode, None).await
}

#[cfg(test)]
pub(crate) async fn execute_sync_with_stats(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
) -> Result<(SyncSummary, SyncExecutionStats)> {
    execute_sync_with_observer_and_stats(cfg, relational, mode, None).await
}

pub(crate) async fn execute_sync_with_observer(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<SyncSummary> {
    let (summary, stats) =
        execute_sync_with_observer_and_stats(cfg, relational, mode, observer).await?;
    stats.log(&cfg.repo.repo_id, &summary.mode);
    Ok(summary)
}

async fn execute_sync_with_observer_and_stats(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<(SyncSummary, SyncExecutionStats)> {
    let (parser_version, extractor_version) = resolve_pack_versions()?;
    let _lock =
        sync::lock::SyncLock::acquire(&cfg.config_root).context("acquiring DevQL sync lock")?;

    sync::lock::write_sync_started(
        relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        sync_reason(&mode),
        &parser_version,
        &extractor_version,
    )
    .await?;

    match execute_sync_inner(
        cfg,
        relational,
        &mode,
        &parser_version,
        &extractor_version,
        observer,
    )
    .await
    {
        Ok((summary, stats)) => {
            sync::lock::write_sync_completed(
                relational,
                &cfg.repo.repo_id,
                summary.head_commit_sha.as_deref(),
                summary.head_tree_sha.as_deref(),
                summary.active_branch.as_deref(),
                &summary.parser_version,
                &summary.extractor_version,
            )
            .await?;
            Ok((summary, stats))
        }
        Err(err) => {
            if let Err(write_err) =
                sync::lock::write_sync_failed(relational, &cfg.repo.repo_id).await
            {
                log::warn!(
                    "failed to mark DevQL sync as failed for repo `{}`: {write_err:#}",
                    cfg.repo.repo_id
                );
            }
            Err(err)
        }
    }
}

async fn execute_sync_inner(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: &sync::types::SyncMode,
    parser_version: &str,
    extractor_version: &str,
    observer: Option<&dyn SyncObserver>,
) -> Result<(SyncSummary, SyncExecutionStats)> {
    let mut counters = sync::types::SyncCounters::default();
    let mut stats = SyncExecutionStats::default();
    let requested_paths = requested_paths(mode);

    emit_progress(
        observer,
        SyncProgressPhase::InspectingWorkspace,
        None,
        &counters,
        0,
        0,
    );
    let workspace_started = Instant::now();
    let workspace =
        sync::workspace_state::inspect_workspace_for_paths(&cfg.repo_root, requested_paths.as_ref())
            .context("inspecting workspace for DevQL sync")?;
    stats.workspace_inspection = workspace_started.elapsed();

    emit_progress(
        observer,
        SyncProgressPhase::BuildingManifest,
        None,
        &counters,
        0,
        0,
    );
    let desired_started = Instant::now();
    let mut desired = sync::manifest::build_desired_manifest(&workspace, &cfg.repo_root, |path| {
        resolve_language_id_for_file_path(path).map(str::to_string)
    })
    .context("building desired manifest for DevQL sync")?;
    if let Some(requested_paths) = requested_paths.as_ref() {
        desired.retain(|path, _| requested_paths.contains(path));
    }
    stats.desired_manifest_build = desired_started.elapsed();

    emit_progress(
        observer,
        SyncProgressPhase::LoadingStoredState,
        None,
        &counters,
        0,
        0,
    );
    let stored_started = Instant::now();
    let stored = load_stored_manifest_for_paths(relational, &cfg.repo.repo_id, requested_paths.as_ref())
        .await
        .context("loading stored manifest for DevQL sync")?;
    stats.stored_manifest_load = stored_started.elapsed();

    let classified = sync::manifest::classify_paths(
        &desired,
        &stored,
        parser_version,
        extractor_version,
        matches!(mode, sync::types::SyncMode::Repair),
    );

    for path in &classified {
        match path.action {
            sync::types::PathAction::Unchanged => counters.paths_unchanged += 1,
            sync::types::PathAction::Added => counters.paths_added += 1,
            sync::types::PathAction::Changed => counters.paths_changed += 1,
            sync::types::PathAction::Removed => counters.paths_removed += 1,
        }
    }

    let paths_total = classified.len();
    let unchanged_total = counters.paths_unchanged;
    let transform_paths_total = counters.paths_added + counters.paths_changed;
    let mut removed_completed = 0usize;
    let mut extracted_completed = 0usize;
    let mut materialized_completed = 0usize;
    let mut paths_completed = estimate_sync_progress_paths_completed(
        unchanged_total,
        removed_completed,
        transform_paths_total,
        extracted_completed,
        materialized_completed,
    );
    emit_progress(
        observer,
        SyncProgressPhase::ClassifyingPaths,
        None,
        &counters,
        paths_total,
        paths_completed,
    );

    let mut writer = SqliteSyncWriter::open(&relational.local.path)
        .await
        .context("opening persistent SQLite sync writer")?;

    let removals = classified
        .iter()
        .filter(|path| matches!(path.action, sync::types::PathAction::Removed))
        .map(|path| PreparedRemoval {
            path: path.path.clone(),
        })
        .collect::<Vec<_>>();
    if !removals.is_empty() {
        emit_progress(
            observer,
            SyncProgressPhase::RemovingPaths,
            removals.first().map(|removal| removal.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
        let remove_started = Instant::now();
        let outcome = writer
            .remove_paths(&cfg.repo.repo_id, &removals)
            .await
            .context("removing stale paths in SQLite sync writer")?;
        stats.materialisation_total += remove_started.elapsed();
        stats.add_writer_commit(outcome.sqlite_commits, outcome.sqlite_rows_written);
        for path in outcome.removed_paths {
            removed_completed += 1;
            paths_completed = estimate_sync_progress_paths_completed(
                unchanged_total,
                removed_completed,
                transform_paths_total,
                extracted_completed,
                materialized_completed,
            );
            emit_progress(
                observer,
                SyncProgressPhase::RemovingPaths,
                Some(path),
                &counters,
                paths_total,
                paths_completed,
            );
        }
    }

    let prepare_inputs = classified
        .iter()
        .enumerate()
        .filter(|(_, path)| {
            matches!(
                path.action,
                sync::types::PathAction::Added | sync::types::PathAction::Changed
            )
        })
        .map(|(index, path)| {
            path.desired
                .clone()
                .map(|desired| (index, desired))
                .ok_or_else(|| anyhow!("missing desired state for sync path `{}`", path.path))
        })
        .collect::<Result<Vec<_>>>()?;

    if !prepare_inputs.is_empty() {
        let worker_count = sync_prepare_worker_count().min(prepare_inputs.len().max(1));
        stats.prepare_worker_count = worker_count;
        let pool = SqliteReadConnectionPool::open(&relational.local.path, worker_count)
            .await
            .context("opening SQLite read connection pool for sync prepare stage")?;
        let cfg = Arc::new(cfg.clone());
        let parser_version = Arc::new(parser_version.to_string());
        let extractor_version = Arc::new(extractor_version.to_string());
        let mut prepare_queue = prepare_inputs.into_iter();
        let mut join_set = JoinSet::new();

        for _ in 0..worker_count {
            if let Some((index, desired)) = prepare_queue.next() {
                join_set.spawn(prepare_sync_item(
                    pool.clone(),
                    Arc::clone(&cfg),
                    desired,
                    index,
                    Arc::clone(&parser_version),
                    Arc::clone(&extractor_version),
                ));
            }
        }

        while !join_set.is_empty() {
            if let Some(deadline) = writer.flush_deadline() {
                tokio::select! {
                    join_result = join_set.join_next() => {
                        let Some(join_result) = join_result else { continue; };
                        let outcome = join_result.context("joining sync prepare task")??;
                        handle_prepared_outcome(
                            &mut counters,
                            &mut stats,
                            &mut writer,
                            outcome,
                            observer,
                            paths_total,
                            unchanged_total,
                            removed_completed,
                            transform_paths_total,
                            &mut extracted_completed,
                            &mut materialized_completed,
                            &mut paths_completed,
                        )?;
                        if let Some((index, desired)) = prepare_queue.next() {
                            join_set.spawn(prepare_sync_item(
                                pool.clone(),
                                Arc::clone(&cfg),
                                desired,
                                index,
                                Arc::clone(&parser_version),
                                Arc::clone(&extractor_version),
                            ));
                        }
                        if writer.should_flush() {
                            flush_pending_materialisations(
                                &mut writer,
                                &cfg.repo.repo_id,
                                parser_version.as_str(),
                                extractor_version.as_str(),
                                &mut stats,
                                observer,
                                &counters,
                                paths_total,
                                unchanged_total,
                                removed_completed,
                                transform_paths_total,
                                extracted_completed,
                                &mut materialized_completed,
                                &mut paths_completed,
                            )
                            .await?;
                        }
                    }
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)), if writer.has_pending_items() => {
                        flush_pending_materialisations(
                            &mut writer,
                            &cfg.repo.repo_id,
                            parser_version.as_str(),
                            extractor_version.as_str(),
                            &mut stats,
                            observer,
                            &counters,
                            paths_total,
                            unchanged_total,
                            removed_completed,
                            transform_paths_total,
                            extracted_completed,
                            &mut materialized_completed,
                            &mut paths_completed,
                        )
                        .await?;
                    }
                }
            } else {
                let Some(join_result) = join_set.join_next().await else {
                    continue;
                };
                let outcome = join_result.context("joining sync prepare task")??;
                handle_prepared_outcome(
                    &mut counters,
                    &mut stats,
                    &mut writer,
                    outcome,
                    observer,
                    paths_total,
                    unchanged_total,
                    removed_completed,
                    transform_paths_total,
                    &mut extracted_completed,
                    &mut materialized_completed,
                    &mut paths_completed,
                )?;
                if let Some((index, desired)) = prepare_queue.next() {
                    join_set.spawn(prepare_sync_item(
                        pool.clone(),
                        Arc::clone(&cfg),
                        desired,
                        index,
                        Arc::clone(&parser_version),
                        Arc::clone(&extractor_version),
                    ));
                }
                if writer.should_flush() {
                    flush_pending_materialisations(
                        &mut writer,
                        &cfg.repo.repo_id,
                        parser_version.as_str(),
                        extractor_version.as_str(),
                        &mut stats,
                        observer,
                        &counters,
                        paths_total,
                        unchanged_total,
                        removed_completed,
                        transform_paths_total,
                        extracted_completed,
                        &mut materialized_completed,
                        &mut paths_completed,
                    )
                    .await?;
                }
            }
        }
    }

    flush_pending_materialisations(
        &mut writer,
        &cfg.repo.repo_id,
        parser_version,
        extractor_version,
        &mut stats,
        observer,
        &counters,
        paths_total,
        unchanged_total,
        removed_completed,
        transform_paths_total,
        extracted_completed,
        &mut materialized_completed,
        &mut paths_completed,
    )
    .await?;

    let touch_started = Instant::now();
    let touch_outcome = writer
        .finish()
        .await
        .context("finalising batched content cache touch updates")?;
    stats.cache_store_total += touch_started.elapsed();
    stats.add_writer_commit(touch_outcome.sqlite_commits, touch_outcome.sqlite_rows_written);

    emit_progress(
        observer,
        SyncProgressPhase::RunningGc,
        None,
        &counters,
        paths_total,
        paths_completed,
    );
    if matches!(
        mode,
        sync::types::SyncMode::Auto | sync::types::SyncMode::Full
    ) {
        let gc_started = Instant::now();
        match writer.run_gc(sync::gc::DEFAULT_GC_TTL_DAYS).await {
            Ok((_gc_result, outcome)) => {
                stats.gc = gc_started.elapsed();
                stats.add_writer_commit(outcome.sqlite_commits, outcome.sqlite_rows_written);
                apply_writer_duration(&mut stats, Duration::ZERO, &outcome);
            }
            Err(err) => {
                stats.gc = gc_started.elapsed();
                log::warn!(
                    "failed to run DevQL cache GC for repo `{}`: {err:#}",
                    cfg.repo.repo_id
                );
            }
        }
    }

    let summary = SyncSummary {
        success: true,
        mode: sync_reason(mode).to_string(),
        parser_version: parser_version.to_string(),
        extractor_version: extractor_version.to_string(),
        active_branch: workspace.active_branch,
        head_commit_sha: workspace.head_commit_sha,
        head_tree_sha: workspace.head_tree_sha,
        paths_unchanged: counters.paths_unchanged,
        paths_added: counters.paths_added,
        paths_changed: counters.paths_changed,
        paths_removed: counters.paths_removed,
        cache_hits: counters.cache_hits,
        cache_misses: counters.cache_misses,
        parse_errors: counters.parse_errors,
        validation: None,
    };
    emit_progress(
        observer,
        SyncProgressPhase::Complete,
        None,
        &counters,
        paths_total,
        paths_total,
    );
    Ok((summary, stats))
}

#[allow(clippy::too_many_arguments)]
fn handle_prepared_outcome(
    counters: &mut sync::types::SyncCounters,
    stats: &mut SyncExecutionStats,
    writer: &mut SqliteSyncWriter,
    outcome: super::sqlite_writer::PreparedSyncOutcome,
    observer: Option<&dyn SyncObserver>,
    paths_total: usize,
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: &mut usize,
    materialized_completed: &mut usize,
    paths_completed: &mut usize,
) -> Result<()> {
    stats.add_prepared_path(&outcome.stats);
    if outcome.cache_hit {
        counters.cache_hits += 1;
    }
    if outcome.cache_miss {
        counters.cache_misses += 1;
    }
    if outcome.parse_error {
        counters.parse_errors += 1;
    }

    *extracted_completed += 1;
    if outcome.prepared_item.is_none() && outcome.parse_error {
        *materialized_completed += 1;
    }
    *paths_completed = estimate_sync_progress_paths_completed(
        unchanged_total,
        removed_completed,
        transform_paths_total,
        *extracted_completed,
        *materialized_completed,
    );
    emit_progress(
        observer,
        SyncProgressPhase::ExtractingPaths,
        Some(outcome.path),
        counters,
        paths_total,
        *paths_completed,
    );

    if let Some(prepared_item) = outcome.prepared_item {
        writer.push_item(prepared_item);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn flush_pending_materialisations(
    writer: &mut SqliteSyncWriter,
    repo_id: &str,
    parser_version: &str,
    extractor_version: &str,
    stats: &mut SyncExecutionStats,
    observer: Option<&dyn SyncObserver>,
    counters: &sync::types::SyncCounters,
    paths_total: usize,
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: usize,
    materialized_completed: &mut usize,
    paths_completed: &mut usize,
) -> Result<()> {
    if !writer.has_pending_items() {
        return Ok(());
    }

    let flush_started = Instant::now();
    let outcome = writer
        .flush(repo_id, parser_version, extractor_version)
        .await
        .context("flushing pending SQLite sync materialisations")?;
    let flush_duration = flush_started.elapsed();
    stats.add_writer_commit(outcome.sqlite_commits, outcome.sqlite_rows_written);
    apply_writer_duration(stats, flush_duration, &outcome);

    for path in outcome.materialized_paths {
        *materialized_completed += 1;
        *paths_completed = estimate_sync_progress_paths_completed(
            unchanged_total,
            removed_completed,
            transform_paths_total,
            extracted_completed,
            *materialized_completed,
        );
        emit_progress(
            observer,
            SyncProgressPhase::MaterialisingPaths,
            Some(path),
            counters,
            paths_total,
            *paths_completed,
        );
    }
    Ok(())
}

fn apply_writer_duration(
    stats: &mut SyncExecutionStats,
    duration: Duration,
    outcome: &WriterCommitOutcome,
) {
    let total_estimate =
        outcome.cache_store_operation_estimate + outcome.materialisation_operation_estimate;
    if total_estimate == 0 {
        return;
    }

    let cache_ratio = outcome.cache_store_operation_estimate as f64 / total_estimate as f64;
    let cache_duration = duration.mul_f64(cache_ratio);
    stats.cache_store_total += cache_duration;
    stats.materialisation_total += duration.checked_sub(cache_duration).unwrap_or_default();
}

fn estimate_sync_progress_paths_completed(
    unchanged_total: usize,
    removed_completed: usize,
    transform_paths_total: usize,
    extracted_completed: usize,
    materialized_completed: usize,
) -> usize {
    let transform_units_total = transform_paths_total.saturating_mul(2);
    let transform_units_completed = extracted_completed
        .saturating_add(materialized_completed)
        .min(transform_units_total);
    let transform_credit = transform_units_completed
        .saturating_add(1)
        .checked_div(2)
        .unwrap_or(0)
        .min(transform_paths_total);

    unchanged_total
        .saturating_add(removed_completed)
        .saturating_add(transform_credit)
}

#[cfg(test)]
mod tests {
    use super::estimate_sync_progress_paths_completed;

    #[test]
    fn extraction_phase_contributes_partial_progress() {
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 1, 0), 1);
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 2, 0), 1);
        assert_eq!(estimate_sync_progress_paths_completed(0, 0, 4, 3, 0), 2);
    }

    #[test]
    fn completed_transform_work_reaches_total_paths() {
        assert_eq!(estimate_sync_progress_paths_completed(5, 2, 4, 4, 4), 11);
    }
}
