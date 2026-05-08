use super::progress::{emit_checkpoint_ingested, emit_progress};
use super::shared::{
    active_branch_name, promote_temporary_current_rows_for_head_commit,
    resolve_pack_versions_for_ingest, tracked_paths_at_revision,
};
use super::*;
pub async fn run_ingest(cfg: &DevqlConfig) -> Result<()> {
    let summary = execute_ingest(cfg).await?;
    println!("{}", format_ingestion_summary(&summary));
    Ok(())
}

pub(crate) async fn execute_ingest(cfg: &DevqlConfig) -> Result<IngestionCounters> {
    execute_ingest_with_observer(cfg, false, 0, None, None).await
}

pub(crate) async fn execute_ingest_with_backfill_window(
    cfg: &DevqlConfig,
    init: bool,
    backfill_window: usize,
    observer: Option<&dyn IngestionObserver>,
    enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    execute_ingest_inner(
        cfg,
        init,
        0,
        Some(backfill_window),
        None,
        observer,
        enrichment,
    )
    .await
}

pub(crate) async fn execute_ingest_with_commits(
    cfg: &DevqlConfig,
    init: bool,
    commits: Vec<String>,
    observer: Option<&dyn IngestionObserver>,
    enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    execute_ingest_inner(cfg, init, 0, None, Some(commits), observer, enrichment).await
}

pub(crate) async fn execute_ingest_with_observer(
    cfg: &DevqlConfig,
    init: bool,
    max_commits: usize,
    observer: Option<&dyn IngestionObserver>,
    enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    execute_ingest_inner(cfg, init, max_commits, None, None, observer, enrichment).await
}

async fn execute_ingest_inner(
    cfg: &DevqlConfig,
    init: bool,
    max_commits: usize,
    backfill_window: Option<usize>,
    explicit_commits: Option<Vec<String>>,
    observer: Option<&dyn IngestionObserver>,
    _enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    let mut counters = IngestionCounters {
        init_requested: init,
        ..IngestionCounters::default()
    };
    let mut encountered_commit_failures = false;
    let mut commits_total = 0usize;
    let mut commits_processed = 0usize;
    emit_progress(
        observer,
        IngestionProgressPhase::Initializing,
        commits_total,
        commits_processed,
        None,
        None,
        &counters,
    );

    let result: Result<()> = async {
    let _ = core_extension_host().context("loading Core extension host for `devql ingest`")?;
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for `devql ingest`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql ingest").await?;

    ensure_repository_row(cfg, &relational).await?;
    let exclusion_matcher = load_repo_exclusion_matcher(&cfg.repo_root)
        .context("loading repo policy exclusions for `devql ingest`")?;
    let (parser_version, extractor_version) = resolve_pack_versions_for_ingest()
        .context("resolving language pack versions for `devql ingest`")?;

    let head_sha = match run_git(&cfg.repo_root, &["rev-parse", "HEAD"]) {
        Ok(sha) => sha,
        Err(err) if is_missing_head_error(&err) => String::new(),
        Err(err) => return Err(err).context("resolving HEAD for commit history ingest"),
    };
    let active_branch = checked_out_branch_name(&cfg.repo_root);
    let _active_branch_for_enqueue = active_branch
        .clone()
        .unwrap_or_else(|| active_branch_name(&cfg.repo_root));
    let mut commits = match explicit_commits {
        Some(commits) => commits,
        None => match backfill_window {
            Some(backfill_window) => {
                select_recent_branch_commit_backfill_window(
                    &cfg.repo_root,
                    &relational,
                    &cfg.repo.repo_id,
                    &head_sha,
                    backfill_window,
                )
                .await?
            }
            None => {
                select_missing_branch_commit_segment(
                    &cfg.repo_root,
                    &relational,
                    &cfg.repo.repo_id,
                    active_branch.as_deref(),
                    &head_sha,
                )
                .await?
            }
        },
    };
    if max_commits > 0 && commits.len() > max_commits {
        commits.truncate(max_commits);
    }
    commits_total = commits.len();
    emit_progress(
        observer,
        IngestionProgressPhase::Initializing,
        commits_total,
        commits_processed,
        None,
        None,
        &counters,
    );

    let checkpoint_mappings = read_commit_checkpoint_mappings(&cfg.repo_root).unwrap_or_default();
    let mut existing_event_ids: Option<std::collections::HashSet<String>> = None;

    for commit_sha in commits {
        let checkpoint_id = checkpoint_mappings.get(&commit_sha).cloned();
        emit_progress(
            observer,
            IngestionProgressPhase::Extracting,
            commits_total,
            commits_processed,
            checkpoint_id.clone(),
            Some(commit_sha.clone()),
            &counters,
        );

        let existing_ledger =
            load_commit_ingest_ledger_entry(&relational, &cfg.repo.repo_id, &commit_sha).await?;
        if existing_ledger.as_ref().is_some_and(commit_is_fully_ingested) {
            if uses_local_ingest_watermarks(&relational)
                && let Some(branch_name) = active_branch.as_deref()
            {
                upsert_sync_state_value(
                    cfg,
                    &relational,
                    &historical_branch_watermark_key(branch_name),
                    &commit_sha,
                )
                .await?;
            }
            counters.commits_processed += 1;
            commits_processed += 1;
            emit_progress(
                observer,
                IngestionProgressPhase::Persisting,
                commits_total,
                commits_processed,
                checkpoint_id.clone(),
                Some(commit_sha.clone()),
                &counters,
            );
            continue;
        }

        let commit_info =
            checkpoint_commit_info_from_sha(&cfg.repo_root, &commit_sha).unwrap_or(
                CheckpointCommitInfo {
                    commit_sha: commit_sha.clone(),
                    commit_unix: 0,
                    author_name: String::new(),
                    author_email: String::new(),
                    subject: String::new(),
                },
            );
        let mut history_completed = existing_ledger
            .as_ref()
            .map(|entry| entry.history_status == "completed")
            .unwrap_or(false);

        let commit_result: Result<()> = async {
            if !history_completed {
                upsert_commit_metadata_row(cfg, &relational, &commit_info).await?;
                let tracked_paths = tracked_paths_at_revision(&cfg.repo_root, &commit_sha)
                    .with_context(|| format!("listing tracked files for commit {commit_sha}"))?;
                let classifier = ProjectAwareClassifier::discover_for_revision(
                    &cfg.repo_root,
                    &commit_sha,
                    tracked_paths,
                    &parser_version,
                    &extractor_version,
                )
                .with_context(|| {
                    format!("building project-aware classifier for commit {commit_sha}")
                })?;
                let mut changed_files = crate::host::checkpoints::strategy::manual_commit::files_changed_in_commit(
                    &cfg.repo_root,
                    &commit_sha,
                )
                .with_context(|| format!("listing changed files for commit {commit_sha}"))?
                .into_iter()
                .collect::<Vec<_>>();
                changed_files.sort();

                for path in changed_files {
                    let normalized_path = normalize_repo_path(&path);
                    if normalized_path.is_empty() {
                        continue;
                    }
                    let excluded_by_policy =
                        exclusion_matcher.excludes_repo_relative_path(&normalized_path);
                    let classification = classifier
                        .classify_repo_relative_path(&normalized_path, excluded_by_policy)
                        .with_context(|| {
                            format!(
                                "classifying historical ingest path `{normalized_path}` at commit {commit_sha}"
                            )
                        })?;
                    if classification.analysis_mode == AnalysisMode::Excluded {
                        continue;
                    }

                    let blob_sha = git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &normalized_path)
                        .or_else(|| git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &path));
                    let Some(blob_sha) = blob_sha else {
                        continue;
                    };
                    let blob_content =
                        git_blob_decoded_content(&cfg.repo_root, &blob_sha).ok_or_else(|| {
                            anyhow!(
                                "failed to decode blob content for historical ingest path `{}` at commit {} (blob {})",
                                normalized_path,
                                commit_sha,
                                blob_sha
                            )
                        })?;

                    upsert_file_state_row(
                        &cfg.repo.repo_id,
                        &relational,
                        &commit_sha,
                        &normalized_path,
                        &blob_sha,
                    )
                    .await?;
                    if !classification.should_extract() {
                        continue;
                    }
                    if classification.analysis_mode == AnalysisMode::Text {
                        let Some(content) = blob_content.text.as_deref() else {
                            continue;
                        };
                        if !plain_text_content_is_allowed(content) {
                            continue;
                        }
                    }
                    let file_artefact = upsert_file_artefact_row(
                        &cfg.repo.repo_id,
                        &relational,
                        &normalized_path,
                        &blob_sha,
                        &classification.language,
                        &classification.extraction_fingerprint,
                        &blob_content,
                    )
                    .await?;
                    if classification.analysis_mode == AnalysisMode::Text {
                        counters.artefacts_upserted += 1;
                        continue;
                    }
                    if blob_content.decode_degraded {
                        counters.artefacts_upserted += 1;
                        continue;
                    }
                    let source_content = blob_content.text.as_deref().unwrap_or_default();
                    upsert_language_artefacts(
                        cfg,
                        &relational,
                        &FileRevision {
                            commit_sha: &commit_sha,
                            revision: TemporalRevisionRef {
                                kind: TemporalRevisionKind::Commit,
                                id: &commit_sha,
                                temp_checkpoint_id: None,
                            },
                            commit_unix: commit_info.commit_unix,
                            path: &normalized_path,
                            blob_sha: &blob_sha,
                        },
                        &file_artefact,
                        source_content,
                    )
                    .await?;
                    counters.artefacts_upserted += 1;
                }

                mark_commit_history_completed(
                    &relational,
                    &cfg.repo.repo_id,
                    &commit_sha,
                    checkpoint_id.as_deref(),
                )
                .await?;
                history_completed = true;
            }

            if let Some(checkpoint_id) = checkpoint_id.as_deref() {
                let checkpoint_completed = existing_ledger
                    .as_ref()
                    .map(|entry| entry.checkpoint_status == "completed")
                    .unwrap_or(false);
                if !checkpoint_completed {
                    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
                        &cfg.repo_root,
                        checkpoint_id,
                    )?
                    .ok_or_else(|| anyhow!("checkpoint mapping exists but metadata is missing for `{checkpoint_id}`"))?;
                    let existing_event_ids = match existing_event_ids.as_mut() {
                        Some(ids) => ids,
                        None => {
                            existing_event_ids =
                                Some(fetch_existing_checkpoint_event_ids(cfg, &backends.events).await?);
                            existing_event_ids
                                .as_mut()
                                .expect("checkpoint event ids must be initialised")
                        }
                    };
                    let event_id = deterministic_uuid(&format!(
                        "{}|{}|{}|checkpoint_committed",
                        cfg.repo.repo_id, checkpoint.checkpoint_id, checkpoint.session_id
                    ));
                    if !existing_event_ids.contains(&event_id) {
                        insert_checkpoint_event(
                            cfg,
                            &backends.events,
                            &checkpoint,
                            &event_id,
                            Some(&commit_info),
                        )
                        .await?;
                        existing_event_ids.insert(event_id);
                        counters.events_inserted += 1;
                    }

                    let _projected_rows = upsert_checkpoint_file_snapshot_rows(
                        cfg,
                        &relational,
                        &checkpoint,
                        &commit_sha,
                        Some(&commit_info),
                    )
                    .await?;

                    mark_commit_checkpoint_completed(
                        &relational,
                        &cfg.repo.repo_id,
                        &commit_sha,
                        Some(checkpoint_id),
                    )
                    .await?;
                    counters.checkpoint_companions_processed += 1;
                    emit_checkpoint_ingested(observer, checkpoint, Some(commit_sha.clone()));
                }
            }

            if uses_local_ingest_watermarks(&relational)
                && let Some(branch_name) = active_branch.as_deref()
            {
                upsert_sync_state_value(
                    cfg,
                    &relational,
                    &historical_branch_watermark_key(branch_name),
                    &commit_sha,
                )
                .await?;
            }

            Ok(())
        }
        .await;

        if let Err(err) = commit_result {
            let _ = mark_commit_ingest_failed(
                &relational,
                &cfg.repo.repo_id,
                &commit_sha,
                checkpoint_id.as_deref(),
                history_completed,
                &format!("{err:#}"),
            )
            .await;
            encountered_commit_failures = true;
            log::warn!(
                "devql ingest skipping failed commit `{}` and continuing: {err:#}",
                commit_sha
            );
            counters.commits_processed += 1;
            commits_processed += 1;
            emit_progress(
                observer,
                IngestionProgressPhase::Persisting,
                commits_total,
                commits_processed,
                checkpoint_id,
                Some(commit_sha),
                &counters,
            );
            continue;
        }

        counters.commits_processed += 1;
        commits_processed += 1;
        emit_progress(
            observer,
            IngestionProgressPhase::Persisting,
            commits_total,
            commits_processed,
            checkpoint_id,
            Some(commit_sha),
            &counters,
        );
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;
    counters.success = !encountered_commit_failures;
    emit_progress(
        observer,
        IngestionProgressPhase::Complete,
        commits_total,
        commits_processed,
        None,
        None,
        &counters,
    );
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(counters),
        Err(err) => {
            emit_progress(
                observer,
                IngestionProgressPhase::Failed,
                commits_total,
                commits_processed,
                None,
                None,
                &counters,
            );
            Err(err)
        }
    }
}
