use super::*;
use crate::capability_packs::semantic_clones::{
    RepoEmbeddingSyncAction, clear_repo_active_embedding_setup,
    determine_repo_embedding_sync_action, load_active_embedding_setup,
    load_current_repo_embedding_states, load_semantic_feature_inputs_for_current_repo,
    persist_active_embedding_setup,
};
use crate::config::{SemanticCloneEmbeddingMode, SemanticSummaryMode};

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
    enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    let mut counters = IngestionCounters {
        init_requested: init,
        ..IngestionCounters::default()
    };
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
    let knowledge_context =
        capability_ingest_context_for_ingester(cfg, None, KNOWLEDGE_CAPABILITY_INGESTER_ID)
            .context("resolving knowledge capability ingester owner")?;
    let capability = resolve_embedding_capability_config_for_repo(&cfg.daemon_config_root);
    let semantic_clones = capability.semantic_clones.clone();
    let preferred_representation_kind =
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code;
    let summary_provider: Arc<dyn semantic::SemanticSummaryProvider> = if enrichment.is_some()
        || semantic_clones.summary_mode == SemanticSummaryMode::Off
    {
        Arc::new(semantic::NoopSemanticSummaryProvider)
    } else {
        match semantic_clones_pack::build_semantic_summary_provider(&semantic_provider_config(cfg)) {
            Ok(provider) => provider,
            Err(err) => {
                log::warn!(
                    "semantic_clones semantic summaries degraded; using deterministic summaries only: {err:#}"
                );
                Arc::new(semantic::NoopSemanticSummaryProvider)
            }
        }
    };
    let embedding_config = embedding_provider_config(cfg);
    for warning in &embedding_config.warnings {
        log::warn!("semantic_clones embeddings config warning: {warning}");
    }
    let embedding_outputs_enabled = semantic_clones.embedding_mode != SemanticCloneEmbeddingMode::Off
        && embedding_config.embedding_profile.is_some();
    let mut embedding_warning = None;
    let embedding_provider = if enrichment.is_none() && embedding_outputs_enabled {
        match semantic_clones_pack::build_symbol_embedding_provider(
            &embedding_config,
            Some(&cfg.repo_root),
        ) {
            Ok(provider) => provider,
            Err(err) => {
                embedding_warning = Some(format!("{err:#}"));
                None
            }
        }
    } else {
        None
    };

    ensure_repository_row(cfg, &relational).await?;

    let mut resolved_embedding_setup = None;
    let direct_embedding_sync_action = if enrichment.is_none() {
        if let Some(embedding_provider) = embedding_provider.as_ref() {
            let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;
            let action = determine_repo_embedding_sync_action(
                &relational,
                &cfg.repo.repo_id,
                preferred_representation_kind,
                &setup,
            )
            .await?;
            resolved_embedding_setup = Some(setup);
            Some(action)
        } else {
            None
        }
    } else {
        None
    };

    let head_sha = match run_git(&cfg.repo_root, &["rev-parse", "HEAD"]) {
        Ok(sha) => sha,
        Err(err) if is_missing_head_error(&err) => String::new(),
        Err(err) => return Err(err).context("resolving HEAD for commit history ingest"),
    };
    let active_branch = checked_out_branch_name(&cfg.repo_root);
    let active_branch_for_enqueue = active_branch
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
    let mut existing_event_ids = fetch_existing_checkpoint_event_ids(cfg, &backends.events).await?;

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
                    if normalized_path.is_empty()
                        || resolve_language_id_for_file_path(&normalized_path).is_none()
                    {
                        continue;
                    }

                    let blob_sha = git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &normalized_path)
                        .or_else(|| git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &path));
                    let Some(blob_sha) = blob_sha else {
                        continue;
                    };
                    let content = git_blob_content(&cfg.repo_root, &blob_sha).unwrap_or_default();

                    upsert_file_state_row(
                        &cfg.repo.repo_id,
                        &relational,
                        &commit_sha,
                        &normalized_path,
                        &blob_sha,
                    )
                    .await?;
                    let file_artefact = upsert_file_artefact_row(
                        &cfg.repo.repo_id,
                        &cfg.repo_root,
                        &relational,
                        &normalized_path,
                        &blob_sha,
                    )
                    .await?;
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
                    )
                    .await?;

                    let pre_stage_artefacts = load_pre_stage_artefacts_for_blob(
                        &relational,
                        &cfg.repo.repo_id,
                        &blob_sha,
                        &normalized_path,
                    )
                    .await?;
                    let pre_stage_dependencies = load_pre_stage_dependencies_for_blob(
                        &relational,
                        &cfg.repo.repo_id,
                        &blob_sha,
                        &normalized_path,
                    )
                    .await?;
                    let semantic_feature_inputs =
                        semantic_clones_pack::build_semantic_feature_inputs(
                            &pre_stage_artefacts,
                            &pre_stage_dependencies,
                            &content,
                        );
                    let input_hashes = semantic_feature_inputs
                        .iter()
                        .map(|input| {
                            (
                                input.artefact_id.clone(),
                                semantic::build_semantic_feature_input_hash(
                                    input,
                                    summary_provider.as_ref(),
                                ),
                            )
                        })
                        .collect::<std::collections::BTreeMap<_, _>>();
                    let semantic_feature_stats = upsert_semantic_feature_rows(
                        &relational,
                        &semantic_feature_inputs,
                        Arc::clone(&summary_provider),
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "running capability ingester `{}` owned by `{}`",
                            knowledge_context.ingester_id, knowledge_context.capability_pack_id
                        )
                    })?;
                    if let Some(enrichment) = enrichment.as_ref() {
                        let enqueue_target = crate::daemon::EnrichmentJobTarget::new(
                            cfg.daemon_config_root.clone(),
                            cfg.repo_root.clone(),
                            cfg.repo.repo_id.clone(),
                            active_branch_for_enqueue.clone(),
                        );
                        if semantic_clones.summary_mode == SemanticSummaryMode::Auto {
                            enrichment
                                .enqueue_semantic_summaries(
                                    enqueue_target.clone(),
                                    semantic_feature_inputs.clone(),
                                    input_hashes.clone(),
                                )
                                .await?;
                        }

                        if embedding_outputs_enabled {
                            enrichment
                                .enqueue_symbol_embeddings(
                                    enqueue_target.clone(),
                                    semantic_feature_inputs.clone(),
                                    input_hashes.clone(),
                                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                                )
                                .await?;
                            enrichment
                                .enqueue_symbol_embeddings(
                                    enqueue_target,
                                    semantic_feature_inputs.clone(),
                                    input_hashes.clone(),
                                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                                )
                                .await?;
                        }
                    } else if let Some(embedding_provider) = embedding_provider.as_ref() {
                        let code_stats = upsert_symbol_embedding_rows(
                            &relational,
                            &semantic_feature_inputs,
                            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                            Arc::clone(embedding_provider),
                        )
                        .await?;
                        counters.symbol_embedding_rows_upserted += code_stats.upserted;
                        counters.symbol_embedding_rows_skipped += code_stats.skipped;
                        let summary_stats = upsert_symbol_embedding_rows(
                            &relational,
                            &semantic_feature_inputs,
                            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                            Arc::clone(embedding_provider),
                        )
                        .await?;
                        counters.symbol_embedding_rows_upserted += summary_stats.upserted;
                        counters.symbol_embedding_rows_skipped += summary_stats.skipped;
                    }
                    counters.artefacts_upserted += 1;
                    counters.semantic_feature_rows_upserted += semantic_feature_stats.upserted;
                    counters.semantic_feature_rows_skipped += semantic_feature_stats.skipped;
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
            return Err(err);
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

    if let Some(enrichment) = enrichment.as_ref()
        && embedding_outputs_enabled
        && counters.artefacts_upserted == 0
        && load_active_embedding_setup(
            &relational,
            &cfg.repo.repo_id,
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        )
            .await?
            .is_none()
    {
        let bootstrap_inputs = load_semantic_feature_inputs_for_current_repo(
            &relational,
            &cfg.repo_root,
            &cfg.repo.repo_id,
        )
        .await?;
        if !bootstrap_inputs.is_empty() {
            let bootstrap_input_hashes = bootstrap_inputs
                .iter()
                .map(|input| {
                    (
                        input.artefact_id.clone(),
                        semantic::build_semantic_feature_input_hash(
                            input,
                            summary_provider.as_ref(),
                        ),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            let enqueue_target = crate::daemon::EnrichmentJobTarget::new(
                cfg.daemon_config_root.clone(),
                cfg.repo_root.clone(),
                cfg.repo.repo_id.clone(),
                active_branch_for_enqueue.clone(),
            );
            enrichment
                .enqueue_symbol_embeddings(
                    enqueue_target.clone(),
                    bootstrap_inputs.clone(),
                    bootstrap_input_hashes.clone(),
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                )
                .await?;
            enrichment
                .enqueue_symbol_embeddings(
                    enqueue_target,
                    bootstrap_inputs,
                    bootstrap_input_hashes,
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                )
                .await?;
        }
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;
    if let Some(warning) = embedding_warning.as_deref() {
        log::warn!("semantic_clones embeddings degraded; skipping embedding and clone stages: {warning}");
    }

    if let (None, Some(embedding_provider)) = (enrichment.as_ref(), embedding_provider.as_ref()) {
        match direct_embedding_sync_action.unwrap_or(RepoEmbeddingSyncAction::Incremental) {
            RepoEmbeddingSyncAction::RefreshCurrentRepo => {
                clear_repo_active_embedding_setup(&relational, &cfg.repo.repo_id).await?;
                crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                    &relational,
                    &cfg.repo.repo_id,
                )
                .await?;
                let current_inputs = load_semantic_feature_inputs_for_current_repo(
                    &relational,
                    &cfg.repo_root,
                    &cfg.repo.repo_id,
                )
                .await?;
                let code_stats = upsert_symbol_embedding_rows(
                    &relational,
                    &current_inputs,
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                    Arc::clone(embedding_provider),
                )
                .await?;
                counters.symbol_embedding_rows_upserted += code_stats.upserted;
                counters.symbol_embedding_rows_skipped += code_stats.skipped;
                let summary_stats = upsert_symbol_embedding_rows(
                    &relational,
                    &current_inputs,
                    crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                    Arc::clone(embedding_provider),
                )
                .await?;
                counters.symbol_embedding_rows_upserted += summary_stats.upserted;
                counters.symbol_embedding_rows_skipped += summary_stats.skipped;
            }
            RepoEmbeddingSyncAction::Incremental | RepoEmbeddingSyncAction::AdoptExisting => {}
        }

        if let Some(setup) = resolved_embedding_setup.as_ref() {
            if let Some(active_state) = select_active_code_embedding_state_for_repo(
                &relational,
                &cfg.repo.repo_id,
                setup,
            )
            .await?
            {
                persist_active_embedding_setup(&relational, &cfg.repo.repo_id, &active_state)
                    .await?;
                let clone_ingest = rebuild_active_clone_edges(cfg, &relational).await?;
                counters.symbol_clone_edges_upserted += clone_ingest.0;
                counters.symbol_clone_sources_scored += clone_ingest.1;
            } else {
                crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
                    &relational,
                    &cfg.repo.repo_id,
                )
                .await?;
            }
        }
    } else if !embedding_outputs_enabled || (enrichment.is_none() && embedding_provider.is_none()) {
        clear_repo_active_embedding_setup(&relational, &cfg.repo.repo_id).await?;
        crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
            &relational,
            &cfg.repo.repo_id,
        )
        .await?;
    }
    counters.success = true;
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

async fn select_active_code_embedding_state_for_repo(
    relational: &RelationalStorage,
    repo_id: &str,
    setup: &embeddings::EmbeddingSetup,
) -> Result<
    Option<
        crate::capability_packs::semantic_clones::embeddings::ActiveEmbeddingRepresentationState,
    >,
> {
    let states = load_current_repo_embedding_states(
        relational,
        repo_id,
        Some(crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code),
    )
    .await?;
    Ok(states.into_iter().find(|state| state.setup == *setup))
}

async fn rebuild_active_clone_edges(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<(usize, usize)> {
    let capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    let clone_ingest = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
            json!({}),
            Some(relational),
        )
        .await
        .with_context(|| {
            format!(
                "running capability ingester `{SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID}` for `{SEMANTIC_CLONES_CAPABILITY_ID}`"
            )
        })?;

    Ok((
        clone_ingest.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        clone_ingest.payload["symbol_clone_sources_scored"]
            .as_u64()
            .unwrap_or_default() as usize,
    ))
}

fn active_branch_name(repo_root: &Path) -> String {
    checked_out_branch_name(repo_root).unwrap_or_else(|| "main".to_string())
}

async fn promote_temporary_current_rows_for_head_commit(
    _cfg: &DevqlConfig,
    _relational: &RelationalStorage,
) -> Result<usize> {
    // Current-state ingestion now writes directly into the sync-shaped tables, so the legacy
    // temporary-row promotion step is intentionally a no-op until a concrete replacement exists.
    Ok(0)
}

fn emit_progress(
    observer: Option<&dyn IngestionObserver>,
    phase: IngestionProgressPhase,
    commits_total: usize,
    commits_processed: usize,
    current_checkpoint_id: Option<String>,
    current_commit_sha: Option<String>,
    counters: &IngestionCounters,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.on_progress(IngestionProgressUpdate {
        phase,
        commits_total,
        commits_processed,
        current_checkpoint_id,
        current_commit_sha,
        counters: counters.clone(),
    });
}

fn emit_checkpoint_ingested(
    observer: Option<&dyn IngestionObserver>,
    checkpoint: crate::host::checkpoints::strategy::manual_commit::CommittedInfo,
    commit_sha: Option<String>,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.on_checkpoint_ingested(IngestedCheckpointNotification {
        checkpoint,
        commit_sha,
    });
}
