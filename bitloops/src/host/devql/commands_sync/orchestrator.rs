use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use tokio::task::JoinSet;

use crate::capability_packs::semantic_clones::RepoEmbeddingSyncAction;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config,
};
use crate::host::capability_host::DevqlCapabilityHost;
use crate::host::capability_host::events::{SyncArtefactDiff, SyncFileDiff};

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
    let (summary, _file_diff, _artefact_diff) =
        run_sync_with_summary_and_observer_and_diffs(cfg, mode, observer).await?;
    Ok(summary)
}

pub async fn run_sync_with_summary_and_observer_and_diffs(
    cfg: &DevqlConfig,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<(SyncSummary, SyncFileDiff, SyncArtefactDiff)> {
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for `devql sync`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql sync").await?;
    if matches!(mode, sync::types::SyncMode::Validate) {
        return match execute_sync_validation(cfg, &relational).await {
            Ok(summary) => Ok((summary, SyncFileDiff::default(), SyncArtefactDiff::default())),
            Err(err) if is_missing_sync_schema_error(&err) => Err(err).context(
                "DevQL sync schema is not initialised. Run `bitloops devql init` before `bitloops devql tasks enqueue --kind sync --validate --status`.",
            ),
            Err(err) => Err(err),
        };
    }

    match execute_sync_with_observer_and_stats_and_diffs(cfg, &relational, mode, observer).await {
        Ok((summary, stats, file_diff, artefact_diff)) => {
            stats.log(&cfg.repo.repo_id, &summary.mode);
            Ok((summary, file_diff, artefact_diff))
        }
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
        execute_sync_with_observer_and_stats_and_diffs(cfg, relational, mode, observer).await?;
    Ok((summary, stats))
}

pub(crate) async fn execute_sync_with_observer_and_stats_and_diffs(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
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
            sync::state::write_sync_completed(
                relational,
                &cfg.repo.repo_id,
                summary.head_commit_sha.as_deref(),
                summary.head_tree_sha.as_deref(),
                summary.active_branch.as_deref(),
                &summary.parser_version,
                &summary.extractor_version,
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
    let current_projection = build_current_projection_context(cfg)?;
    let mut current_projection_changed = false;
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
            sync::semantic_projector::remove_path(cfg, relational, path)
                .await
                .with_context(|| {
                    format!("removing current semantic clone projection for `{path}`")
                })?;
        }
        if !outcome.removed_paths.is_empty() {
            current_projection_changed = true;
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
                                cfg.as_ref(),
                                relational,
                                &current_projection,
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
                                &mut current_projection_changed,
                            )
                            .await?;
                        }
                    }
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)), if writer.has_pending_items() => {
                        flush_pending_materialisations(
                            cfg.as_ref(),
                            relational,
                            &current_projection,
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
                            &mut current_projection_changed,
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
                        cfg.as_ref(),
                        relational,
                        &current_projection,
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
                        &mut current_projection_changed,
                    )
                    .await?;
                }
            }
        }
    }

    flush_pending_materialisations(
        cfg,
        relational,
        &current_projection,
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
        &mut current_projection_changed,
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

    finalize_semantic_clone_projection_after_sync(
        cfg,
        relational,
        &current_projection,
        current_projection_changed,
    )
    .await?;

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
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    current_projection: &DevqlCapabilityHost,
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
    current_projection_changed: &mut bool,
) -> Result<()> {
    if !writer.has_pending_items() {
        return Ok(());
    }

    let flush_started = Instant::now();
    let outcome = writer
        .flush(repo_id, parser_version, extractor_version)
        .await
        .context("flushing pending SQLite sync materialisations")?;
    if !outcome.materialized_items.is_empty() {
        project_materialized_items(
            cfg,
            relational,
            current_projection,
            &outcome.materialized_items,
        )
        .await
        .context("projecting current semantic clone rows for synced paths")?;
        *current_projection_changed = true;
    }
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

fn build_current_projection_context(cfg: &DevqlConfig) -> Result<DevqlCapabilityHost> {
    build_capability_host(&cfg.repo_root, cfg.repo.clone())
}

async fn project_materialized_items(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    current_projection: &DevqlCapabilityHost,
    items: &[super::sqlite_writer::PreparedSyncItem],
) -> Result<()> {
    for item in items {
        let inputs =
            semantic_features::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                &sync::semantic_projector::pre_stage_artefacts_for_projection(
                    cfg,
                    &item.desired,
                    &item.extraction,
                )?,
                &sync::semantic_projector::pre_stage_dependencies_for_projection(
                    cfg,
                    &item.desired,
                    &item.extraction,
                )?,
                &item.effective_content,
            );
        current_projection
            .invoke_ingester_with_relational(
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
                serde_json::to_value(
                    crate::capability_packs::semantic_clones::ingesters::SemanticFeaturesRefreshPayload {
                        scope: crate::capability_packs::semantic_clones::ingesters::SemanticFeaturesRefreshScope::CurrentPath,
                        path: Some(item.desired.path.clone()),
                        content_id: Some(item.desired.effective_content_id.clone()),
                        inputs: inputs.clone(),
                        mode: crate::capability_packs::semantic_clones::ingesters::SemanticSummaryRefreshMode::ConfiguredDegrade,
                    }
                )?,
                Some(relational),
            )
            .await
            .with_context(|| format!("refreshing current semantic features for `{}`", item.desired.path))?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &item.desired.path,
            &item.desired.effective_content_id,
            &inputs,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing current code embeddings for `{}`",
                item.desired.path
            )
        })?;
        refresh_current_path_embeddings(
            current_projection,
            relational,
            &item.desired.path,
            &item.desired.effective_content_id,
            &inputs,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing current summary embeddings for `{}`",
                item.desired.path
            )
        })?;
    }

    Ok(())
}

async fn refresh_current_path_embeddings(
    current_projection: &DevqlCapabilityHost,
    relational: &RelationalStorage,
    path: &str,
    content_id: &str,
    inputs: &[semantic_features::SemanticFeatureInput],
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    current_projection
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(
                crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshPayload {
                    scope: crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshScope::CurrentPath,
                    path: Some(path.to_string()),
                    content_id: Some(content_id.to_string()),
                    inputs: inputs.to_vec(),
                    expected_input_hashes: Default::default(),
                    representation_kind,
                    mode: crate::capability_packs::semantic_clones::ingesters::EmbeddingRefreshMode::ConfiguredDegrade,
                    manage_active_state: false,
                }
            )?,
            Some(relational),
        )
        .await?;
    Ok(())
}

async fn finalize_semantic_clone_projection_after_sync(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    capability_host: &DevqlCapabilityHost,
    current_projection_changed: bool,
) -> Result<()> {
    let semantic_clones = resolve_semantic_clones_config(
        &capability_host
            .config_view(crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID),
    );
    if !embeddings_enabled(&semantic_clones) {
        clear_semantic_clone_embedding_outputs(relational, &cfg.repo.repo_id).await?;
        return Ok(());
    }

    let semantic_inference = capability_host.inference_for_capability(
        crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
    );
    let mut current_inputs: Option<Vec<semantic_features::SemanticFeatureInput>> = None;
    let mut code_setup = None;
    let mut rebuilt_current_projection = false;
    let mut rebuilt_historical_projection = false;

    for representation_kind in [
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
    ] {
        let selection = resolve_embedding_provider(
            &semantic_clones,
            &semantic_inference,
            representation_kind,
            EmbeddingProviderMode::ConfiguredDegrade,
        )?;
        if let Some(reason) = selection.degraded_reason.as_deref() {
            log::warn!(
                "semantic_clones {} embeddings degraded during sync finalization for repo `{}`: {}",
                representation_kind,
                cfg.repo.repo_id,
                reason
            );
        }
        let Some(provider) = selection.provider.as_ref() else {
            clear_semantic_clone_embedding_outputs_for_representation(
                relational,
                &cfg.repo.repo_id,
                representation_kind,
            )
            .await?;
            if representation_kind
                == crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
            {
                crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                    relational,
                    &cfg.repo.repo_id,
                )
                .await?;
                crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
                    relational,
                    &cfg.repo.repo_id,
                )
                .await?;
            }
            continue;
        };

        let setup = crate::capability_packs::semantic_clones::embeddings::resolve_embedding_setup(
            provider.as_ref(),
        )?;
        if representation_kind
            == crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code
        {
            code_setup = Some(setup.clone());
        }
        let sync_action =
            crate::capability_packs::semantic_clones::determine_repo_embedding_sync_action(
                relational,
                &cfg.repo.repo_id,
                representation_kind,
                &setup,
            )
            .await?;
        let should_refresh_repo_embeddings =
            current_projection_changed || sync_action != RepoEmbeddingSyncAction::Incremental;
        if !should_refresh_repo_embeddings {
            continue;
        }

        let inputs = if let Some(inputs) = current_inputs.as_ref() {
            inputs.clone()
        } else {
            let loaded =
                crate::capability_packs::semantic_clones::load_semantic_feature_inputs_for_current_repo(
                    relational,
                    &cfg.repo_root,
                    &cfg.repo.repo_id,
                )
                .await?;
            current_inputs = Some(loaded.clone());
            loaded
        };
        if inputs.is_empty() {
            continue;
        }

        let outcome = run_repo_symbol_embeddings_refresh(
            capability_host,
            relational,
            representation_kind,
            inputs,
        )
        .await
        .with_context(|| {
            format!(
                "refreshing repo-wide {} embeddings after sync",
                representation_kind
            )
        })?;
        rebuilt_current_projection |= outcome.symbol_clone_edges_upserted > 0
            || sync_action == RepoEmbeddingSyncAction::RefreshCurrentRepo;
        rebuilt_historical_projection |= outcome.clone_rebuild_recommended
            || outcome.symbol_clone_edges_upserted > 0
            || sync_action != RepoEmbeddingSyncAction::Incremental;
    }

    if let Some(setup) = code_setup.as_ref()
        && rebuilt_historical_projection
    {
        if let Some(active_state) =
            select_active_code_embedding_state_for_repo(relational, &cfg.repo.repo_id, setup)
                .await?
        {
            crate::capability_packs::semantic_clones::persist_active_embedding_setup(
                relational,
                &cfg.repo.repo_id,
                &active_state,
            )
            .await?;
            rebuild_active_clone_edges(capability_host, relational).await?;
            rebuilt_current_projection = true;
        } else {
            crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                relational,
                &cfg.repo.repo_id,
            )
            .await?;
        }
    }

    if current_projection_changed || rebuilt_current_projection {
        crate::capability_packs::semantic_clones::pipeline::rebuild_current_symbol_clone_edges(
            relational,
            &cfg.repo.repo_id,
        )
        .await
        .context("rebuilding current semantic clone edges after sync projection changes")?;
    }

    Ok(())
}

async fn clear_semantic_clone_embedding_outputs(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows(relational, repo_id)
        .await?;
    crate::capability_packs::semantic_clones::clear_repo_active_embedding_setup(
        relational, repo_id,
    )
    .await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational, repo_id,
    )
    .await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_current_symbol_clone_edges(
        relational, repo_id,
    )
    .await
}

async fn clear_semantic_clone_embedding_outputs_for_representation(
    relational: &RelationalStorage,
    repo_id: &str,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
) -> Result<()> {
    crate::capability_packs::semantic_clones::clear_repo_symbol_embedding_rows_for_representation(
        relational,
        repo_id,
        representation_kind,
    )
    .await?;
    crate::capability_packs::semantic_clones::clear_repo_active_embedding_setup_for_representation(
        relational,
        repo_id,
        representation_kind,
    )
    .await
}

#[derive(Debug, Clone, Default)]
struct SyncSymbolEmbeddingsRefreshOutcome {
    clone_rebuild_recommended: bool,
    symbol_clone_edges_upserted: usize,
}

async fn run_repo_symbol_embeddings_refresh(
    capability_host: &DevqlCapabilityHost,
    relational: &RelationalStorage,
    representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind,
    inputs: Vec<semantic_features::SemanticFeatureInput>,
) -> Result<SyncSymbolEmbeddingsRefreshOutcome> {
    let result = capability_host
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(
                crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshPayload {
                    scope: crate::capability_packs::semantic_clones::ingesters::SymbolEmbeddingsRefreshScope::Historical,
                    path: None,
                    content_id: None,
                    inputs,
                    expected_input_hashes: Default::default(),
                    representation_kind,
                    mode: crate::capability_packs::semantic_clones::ingesters::EmbeddingRefreshMode::ConfiguredDegrade,
                    manage_active_state: true,
                },
            )?,
            Some(relational),
        )
        .await?;
    Ok(SyncSymbolEmbeddingsRefreshOutcome {
        clone_rebuild_recommended: result.payload["clone_rebuild_recommended"]
            .as_bool()
            .unwrap_or(false),
        symbol_clone_edges_upserted: result.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
    })
}

async fn select_active_code_embedding_state_for_repo(
    relational: &RelationalStorage,
    repo_id: &str,
    setup: &crate::capability_packs::semantic_clones::embeddings::EmbeddingSetup,
) -> Result<
    Option<
        crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState,
    >,
> {
    let states = crate::capability_packs::semantic_clones::load_current_repo_embedding_states(
        relational,
        repo_id,
        Some(
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        ),
    )
    .await?;
    Ok(states.into_iter().find(|state| state.setup == *setup))
}

async fn rebuild_active_clone_edges(
    capability_host: &DevqlCapabilityHost,
    relational: &RelationalStorage,
) -> Result<()> {
    capability_host
        .invoke_ingester_with_relational(
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            serde_json::json!({}),
            Some(relational),
        )
        .await
        .with_context(|| {
            format!(
                "running capability ingester `{}` for `{}`",
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID
            )
        })?;
    Ok(())
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
