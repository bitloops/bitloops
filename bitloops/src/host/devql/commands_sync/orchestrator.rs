use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::task::JoinSet;

use super::diff_collector::SyncDiffCollector;
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
use crate::host::capability_host::events::{SyncArtefactDiff, SyncFileDiff};

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
    let (summary, stats, _file_diff, _artefact_diff) =
        run_sync_with_summary_and_stats_and_observer_and_diffs(cfg, mode, observer, None).await?;
    stats.log(&cfg.repo.repo_id, &summary.mode);
    Ok(summary)
}

pub async fn run_sync_with_summary_and_observer_and_diffs(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<(SyncSummary, SyncFileDiff, SyncArtefactDiff)> {
    let (summary, _stats, file_diff, artefact_diff) =
        run_sync_with_summary_and_stats_and_observer_and_diffs(cfg, mode, observer, None).await?;
    Ok((summary, file_diff, artefact_diff))
}

pub(crate) async fn run_sync_with_summary_and_stats_and_observer_and_diffs(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
    scope_exclusions_fingerprint_override: Option<&str>,
) -> Result<(
    SyncSummary,
    SyncExecutionStats,
    SyncFileDiff,
    SyncArtefactDiff,
)> {
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for `devql sync`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql sync").await?;
    if matches!(mode, sync::types::SyncMode::Validate) {
        return match execute_sync_validation(cfg, &relational).await {
            Ok(summary) => Ok((
                summary,
                SyncExecutionStats::default(),
                SyncFileDiff::default(),
                SyncArtefactDiff::default(),
            )),
            Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
                "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql tasks enqueue --kind sync --validate --status`.",
            ),
            Err(err) => Err(err),
        };
    }

    match execute_sync_with_observer_and_stats_and_diffs(
        cfg,
        &relational,
        mode,
        observer,
        scope_exclusions_fingerprint_override,
    )
    .await
    {
        Ok((summary, stats, file_diff, artefact_diff)) => Ok((summary, stats, file_diff, artefact_diff)),
        Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
            "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql tasks enqueue --kind sync --status`.",
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
    let (summary, stats, _file_diff, _artefact_diff) =
        execute_sync_with_observer_and_stats_and_diffs(cfg, relational, mode, observer, None)
            .await?;
    Ok((summary, stats))
}

pub(crate) async fn execute_sync_with_observer_and_stats_and_diffs(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
    scope_exclusions_fingerprint_override: Option<&str>,
) -> Result<(
    SyncSummary,
    SyncExecutionStats,
    SyncFileDiff,
    SyncArtefactDiff,
)> {
    let (parser_version, extractor_version) = resolve_pack_versions()?;

    ensure_repository_row(cfg, relational)
        .await
        .context("ensuring repository catalog row for DevQL sync")?;

    sync::state::write_sync_started(
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
        Ok((summary, stats, file_diff, artefact_diff)) => {
            let scope_exclusions_fingerprint = scope_exclusions_fingerprint_override
                .map(str::to_string)
                .map(Ok)
                .unwrap_or_else(|| {
                    crate::host::devql::current_scope_exclusions_fingerprint(&cfg.repo_root)
                        .context(
                            "loading current scope exclusions fingerprint for completed DevQL sync",
                        )
                })?;
            sync::state::write_sync_completed(
                relational,
                &cfg.repo.repo_id,
                sync::state::SyncCompletionState {
                    head_commit_sha: summary.head_commit_sha.as_deref(),
                    head_tree_sha: summary.head_tree_sha.as_deref(),
                    active_branch: summary.active_branch.as_deref(),
                    parser_version: &summary.parser_version,
                    extractor_version: &summary.extractor_version,
                    scope_exclusions_fingerprint: &scope_exclusions_fingerprint,
                },
            )
            .await?;
            Ok((summary, stats, file_diff, artefact_diff))
        }
        Err(err) => {
            if let Err(write_err) =
                sync::state::write_sync_failed(relational, &cfg.repo.repo_id).await
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
) -> Result<(
    SyncSummary,
    SyncExecutionStats,
    SyncFileDiff,
    SyncArtefactDiff,
)> {
    let mut counters = sync::types::SyncCounters::default();
    let mut stats = SyncExecutionStats::default();
    let mut diff_collector = SyncDiffCollector::new();
    let exclusion_matcher = load_repo_exclusion_matcher(&cfg.repo_root)
        .context("loading repo policy exclusions for DevQL sync")?;
    let internal_ignored_paths = sync_internal_ignored_paths(relational, &cfg.repo_root);
    let mut requested_paths = requested_paths(mode);
    if let Some(paths) = requested_paths.as_mut() {
        paths.retain(|path| !exclusion_matcher.excludes_repo_relative_path(path));
        paths.retain(|path| !is_sync_internal_ignored_path(path, &internal_ignored_paths));
    }

    emit_progress(
        observer,
        SyncProgressPhase::InspectingWorkspace,
        None,
        &counters,
        0,
        0,
    );
    let workspace_started = Instant::now();
    let mut workspace = sync::workspace_state::inspect_workspace_for_paths(
        &cfg.repo_root,
        requested_paths.as_ref(),
    )
    .context("inspecting workspace for DevQL sync")?;
    workspace.head_tree.retain(|path, _| {
        !exclusion_matcher.excludes_repo_relative_path(path)
            && !is_sync_internal_ignored_path(path, &internal_ignored_paths)
    });
    workspace.staged_changes.retain(|path, _| {
        !exclusion_matcher.excludes_repo_relative_path(path)
            && !is_sync_internal_ignored_path(path, &internal_ignored_paths)
    });
    workspace.dirty_files.retain(|path| {
        !exclusion_matcher.excludes_repo_relative_path(path)
            && !is_sync_internal_ignored_path(path, &internal_ignored_paths)
    });
    workspace.untracked_files.retain(|path| {
        !exclusion_matcher.excludes_repo_relative_path(path)
            && !is_sync_internal_ignored_path(path, &internal_ignored_paths)
    });
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
    let classifier = ProjectAwareClassifier::discover_for_worktree(
        &cfg.repo_root,
        workspace
            .head_tree
            .keys()
            .cloned()
            .chain(workspace.staged_changes.keys().cloned())
            .chain(workspace.dirty_files.iter().cloned())
            .chain(workspace.untracked_files.iter().cloned())
            .collect::<Vec<_>>(),
        parser_version,
        extractor_version,
    )
    .context("building project-aware classifier for DevQL sync")?;
    replace_project_contexts_current(relational, &cfg.repo.repo_id, &classifier.contexts())
        .await
        .context("persisting project context snapshot for DevQL sync")?;
    let mut desired = sync::manifest::build_desired_manifest(&workspace, &cfg.repo_root, |path| {
        let classification = classifier.classify_repo_relative_path(path, false)?;
        if !classification.should_persist_current_state() {
            return Ok(None);
        }
        Ok(Some(classification))
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
    let stored =
        load_stored_manifest_for_paths(relational, &cfg.repo.repo_id, requested_paths.as_ref())
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
            sync::types::PathAction::Unchanged => {
                counters.paths_unchanged += 1;
            }
            sync::types::PathAction::Added => {
                counters.paths_added += 1;
                if let Some(desired) = &path.desired {
                    diff_collector.record_file_added(
                        path.path.clone(),
                        desired.language.clone(),
                        desired.effective_content_id.clone(),
                    );
                }
            }
            sync::types::PathAction::Changed => {
                counters.paths_changed += 1;
                if let Some(desired) = &path.desired {
                    diff_collector.record_file_changed(
                        path.path.clone(),
                        desired.language.clone(),
                        desired.effective_content_id.clone(),
                    );
                }
            }
            sync::types::PathAction::Removed => {
                counters.paths_removed += 1;
                diff_collector.record_file_removed(path.path.clone());
            }
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

    let mut writer = SqliteSyncWriter::open(relational.sqlite_path())
        .await
        .context("opening persistent SQLite sync writer")?;
    let mut touched_paths = HashSet::<String>::new();
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
        for path in &outcome.removed_paths {
            touched_paths.insert(path.clone());
        }
        for artefact in outcome.pre_artefacts.clone() {
            diff_collector.record_pre_artefacts(artefact.path.clone(), vec![artefact]);
        }
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
        let pool = SqliteReadConnectionPool::open(relational.sqlite_path(), worker_count)
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
                        let outcome = join_result.context("joining sync prepare task")?;
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
                                &mut diff_collector,
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
                                &mut touched_paths,
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
                            &mut diff_collector,
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
                            &mut touched_paths,
                        )
                        .await?;
                    }
                }
            } else {
                let Some(join_result) = join_set.join_next().await else {
                    continue;
                };
                let outcome = join_result.context("joining sync prepare task")?;
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
                        &mut diff_collector,
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
                        &mut touched_paths,
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
        &mut diff_collector,
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
        &mut touched_paths,
    )
    .await?;

    let touch_started = Instant::now();
    let touch_outcome = writer
        .finish()
        .await
        .context("finalising batched content cache touch updates")?;
    stats.cache_store_total += touch_started.elapsed();
    stats.add_writer_commit(
        touch_outcome.sqlite_commits,
        touch_outcome.sqlite_rows_written,
    );

    let touched_paths = touched_paths.into_iter().collect::<Vec<_>>();
    let reconcile_started = Instant::now();
    sync::materializer::reconcile_current_local_edges_for_paths(
        relational,
        &cfg.repo.repo_id,
        &touched_paths,
    )
    .await
    .context("reconciling current local dependency edges after sync")?;
    stats.current_edge_reconcile_total = reconcile_started.elapsed();

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
    let (file_diff, artefact_diff) = diff_collector.into_diffs();
    Ok((summary, stats, file_diff, artefact_diff))
}

async fn replace_project_contexts_current(
    relational: &RelationalStorage,
    repo_id: &str,
    contexts: &[ProjectContext],
) -> Result<()> {
    let mut statements = vec![format!(
        "DELETE FROM project_contexts_current WHERE repo_id = '{}'",
        esc_pg(repo_id),
    )];
    statements.extend(contexts.iter().map(|context| {
        format!(
            "INSERT INTO project_contexts_current (repo_id, context_id, root, kind, detection_source, frameworks_json, runtime_profile, config_files_json, config_fingerprint, source_versions_json) \
VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', '{}')",
            esc_pg(repo_id),
            esc_pg(&context.context_id),
            esc_pg(&context.root),
            esc_pg(&context.kind),
            esc_pg(&context.detection_source),
            esc_pg(
                &serde_json::to_string(&context.frameworks)
                    .unwrap_or_else(|_| "[]".to_string()),
            ),
            context
                .runtime_profile
                .as_deref()
                .map(|value| format!("'{}'", esc_pg(value)))
                .unwrap_or_else(|| "NULL".to_string()),
            esc_pg(
                &serde_json::to_string(&context.config_files)
                    .unwrap_or_else(|_| "[]".to_string()),
            ),
            esc_pg(&context.config_fingerprint),
            esc_pg(
                &serde_json::to_string(&context.source_versions)
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
        )
    }));
    relational.exec_batch_transactional(&statements).await
}

fn sync_internal_ignored_paths(
    relational: &RelationalStorage,
    repo_root: &Path,
) -> HashSet<String> {
    let mut ignored = HashSet::new();
    let sqlite_path = relational.sqlite_path();
    let Some(relative) = sqlite_path
        .strip_prefix(repo_root)
        .ok()
        .map(|path| normalize_repo_path(path.to_string_lossy().as_ref()))
        .filter(|path| !path.is_empty())
    else {
        return ignored;
    };

    ignored.insert(relative.clone());
    ignored
}

fn is_sync_internal_ignored_path(path: &str, ignored: &HashSet<String>) -> bool {
    if path.starts_with(".bitloops/stores/") {
        return true;
    }
    ignored
        .iter()
        .any(|base| path == base || path.starts_with(&format!("{base}-")))
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
    if let Some(error_message) = outcome.error_message.as_deref() {
        log::warn!(
            "skipping sync path `{}` due prepare failure: {error_message}",
            outcome.path
        );
    }
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
    diff_collector: &mut SyncDiffCollector,
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
    touched_paths: &mut HashSet<String>,
) -> Result<()> {
    if !writer.has_pending_items() {
        return Ok(());
    }

    let flush_started = Instant::now();
    let outcome = writer
        .flush(repo_id, parser_version, extractor_version)
        .await
        .context("flushing pending SQLite sync materialisations")?;
    for artefact in outcome.pre_artefacts.clone() {
        diff_collector.record_pre_artefacts(artefact.path.clone(), vec![artefact]);
    }
    for artefact in outcome.post_artefacts.clone() {
        diff_collector.record_post_artefacts(artefact.path.clone(), vec![artefact]);
    }
    let flush_duration = flush_started.elapsed();
    stats.add_writer_commit(outcome.sqlite_commits, outcome.sqlite_rows_written);
    apply_writer_duration(stats, flush_duration, &outcome);

    for path in outcome.materialized_paths {
        touched_paths.insert(path.clone());
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
