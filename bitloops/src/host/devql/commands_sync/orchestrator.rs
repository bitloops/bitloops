use anyhow::{Context, Result, anyhow};

use super::progress::{SyncObserver, SyncProgressPhase, emit_progress};
use super::shared::{
    determine_retention_class, is_missing_sync_schema_error, load_stored_manifest,
    read_effective_content, requested_paths, resolve_pack_versions, sync_reason,
};
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

pub(crate) async fn execute_sync_with_observer(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    mode: sync::types::SyncMode,
    observer: Option<&dyn SyncObserver>,
) -> Result<SyncSummary> {
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
        Ok(summary) => {
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
            Ok(summary)
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
) -> Result<SyncSummary> {
    let mut counters = sync::types::SyncCounters::default();

    // Phase 0: inspect the current workspace and resolve any path scope.
    emit_progress(
        observer,
        SyncProgressPhase::InspectingWorkspace,
        None,
        &counters,
        0,
        0,
    );
    let workspace = sync::workspace_state::inspect_workspace(&cfg.repo_root)
        .context("inspecting workspace for DevQL sync")?;
    let requested_paths = requested_paths(mode);

    // Phase 1: build the desired manifest for supported source files only.
    emit_progress(
        observer,
        SyncProgressPhase::BuildingManifest,
        None,
        &counters,
        0,
        0,
    );
    let mut desired = sync::manifest::build_desired_manifest(&workspace, &cfg.repo_root, |path| {
        resolve_language_id_for_file_path(path).map(str::to_string)
    })
    .context("building desired manifest for DevQL sync")?;
    if let Some(requested_paths) = requested_paths.as_ref() {
        desired.retain(|path, _| requested_paths.contains(path));
    }

    // Phase 2: load the stored manifest from current sync state.
    emit_progress(
        observer,
        SyncProgressPhase::LoadingStoredState,
        None,
        &counters,
        0,
        0,
    );
    let mut stored = load_stored_manifest(relational, &cfg.repo.repo_id)
        .await
        .context("loading stored manifest for DevQL sync")?;
    if let Some(requested_paths) = requested_paths.as_ref() {
        stored.retain(|path, _| requested_paths.contains(path));
    }

    // Phase 3: classify path changes for this sync run.
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

    // Phase 4: remove paths that no longer exist in the effective manifest.
    for path in classified
        .iter()
        .filter(|path| matches!(path.action, sync::types::PathAction::Removed))
    {
        emit_progress(
            observer,
            SyncProgressPhase::RemovingPaths,
            Some(path.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
        sync::materializer::remove_path(cfg, relational, &path.path)
            .await
            .with_context(|| format!("removing stale path `{}` during DevQL sync", path.path))?;
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
            Some(path.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
    }

    // Phase 5: hydrate cached extraction payloads or re-read effective content.
    let mut staged_materializations = Vec::new();
    for path in classified.iter().filter(|path| {
        matches!(
            path.action,
            sync::types::PathAction::Added | sync::types::PathAction::Changed
        )
    }) {
        emit_progress(
            observer,
            SyncProgressPhase::ExtractingPaths,
            Some(path.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
        let desired = path
            .desired
            .clone()
            .ok_or_else(|| anyhow!("missing desired state for sync path `{}`", path.path))?;

        let extraction = match sync::content_cache::lookup_cached_content(
            relational,
            &desired.effective_content_id,
            &desired.language,
            parser_version,
            extractor_version,
        )
        .await
        .with_context(|| format!("looking up cached extraction for `{}`", desired.path))?
        {
            Some(cached) => {
                counters.cache_hits += 1;
                if cached.parse_status == sync::extraction::PARSE_STATUS_PARSE_ERROR {
                    counters.parse_errors += 1;
                }
                if determine_retention_class(&desired) == "git_backed" {
                    sync::content_cache::promote_to_git_backed(
                        relational,
                        &desired.effective_content_id,
                        &desired.language,
                        parser_version,
                        extractor_version,
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "promoting cached extraction retention for `{}`",
                            desired.path
                        )
                    })?;
                }
                cached
            }
            None => {
                counters.cache_misses += 1;
                let content = read_effective_content(cfg, &desired)
                    .with_context(|| format!("reading effective content for `{}`", desired.path))?;
                let Some(extraction) = sync::extraction::extract_to_cache_format(
                    cfg,
                    &desired.path,
                    &desired.effective_content_id,
                    parser_version,
                    extractor_version,
                    &content,
                )
                .with_context(|| format!("extracting `{}` into sync cache format", desired.path))?
                else {
                    counters.parse_errors += 1;
                    extracted_completed += 1;
                    materialized_completed += 1;
                    paths_completed = estimate_sync_progress_paths_completed(
                        unchanged_total,
                        removed_completed,
                        transform_paths_total,
                        extracted_completed,
                        materialized_completed,
                    );
                    emit_progress(
                        observer,
                        SyncProgressPhase::ExtractingPaths,
                        Some(path.path.clone()),
                        &counters,
                        paths_total,
                        paths_completed,
                    );
                    continue;
                };

                sync::content_cache::store_cached_content(
                    relational,
                    &extraction,
                    determine_retention_class(&desired),
                )
                .await
                .with_context(|| format!("storing cached extraction for `{}`", desired.path))?;
                extraction
            }
        };

        extracted_completed += 1;
        paths_completed = estimate_sync_progress_paths_completed(
            unchanged_total,
            removed_completed,
            transform_paths_total,
            extracted_completed,
            materialized_completed,
        );
        emit_progress(
            observer,
            SyncProgressPhase::ExtractingPaths,
            Some(path.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
        staged_materializations.push((desired, extraction));
    }

    // Phase 6: materialize added and changed paths into current-state tables.
    for (desired, extraction) in staged_materializations {
        emit_progress(
            observer,
            SyncProgressPhase::MaterialisingPaths,
            Some(desired.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
        sync::materializer::materialize_path(
            cfg,
            relational,
            &desired,
            &extraction,
            parser_version,
            extractor_version,
        )
        .await
        .with_context(|| format!("materializing `{}` during DevQL sync", desired.path))?;
        materialized_completed += 1;
        paths_completed = estimate_sync_progress_paths_completed(
            unchanged_total,
            removed_completed,
            transform_paths_total,
            extracted_completed,
            materialized_completed,
        );
        emit_progress(
            observer,
            SyncProgressPhase::MaterialisingPaths,
            Some(desired.path.clone()),
            &counters,
            paths_total,
            paths_completed,
        );
    }

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
    ) && let Err(err) =
        sync::gc::run_gc(relational, &cfg.repo.repo_id, sync::gc::DEFAULT_GC_TTL_DAYS).await
    {
        log::warn!(
            "failed to run DevQL cache GC for repo `{}`: {err:#}",
            cfg.repo.repo_id
        );
    }

    // Phase 7: emit the final summary with the resolved workspace identity.
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
    Ok(summary)
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
