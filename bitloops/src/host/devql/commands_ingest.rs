use super::*;
use crate::config::{SemanticCloneEmbeddingMode, SemanticSummaryMode};

pub async fn run_ingest(cfg: &DevqlConfig, max_checkpoints: usize) -> Result<()> {
    let summary = execute_ingest(cfg, max_checkpoints).await?;
    println!("{}", format_ingestion_summary(&summary));
    Ok(())
}

pub(crate) async fn execute_ingest(
    cfg: &DevqlConfig,
    max_checkpoints: usize,
) -> Result<IngestionCounters> {
    execute_ingest_with_observer(cfg, max_checkpoints, None, None).await
}

pub(crate) async fn execute_ingest_with_observer(
    cfg: &DevqlConfig,
    max_checkpoints: usize,
    observer: Option<&dyn IngestionObserver>,
    enrichment: Option<Arc<crate::daemon::EnrichmentCoordinator>>,
) -> Result<IngestionCounters> {
    let mut counters = IngestionCounters::default();
    let mut checkpoints_total = 0usize;
    let mut checkpoints_processed = 0usize;
    emit_progress(
        observer,
        IngestionProgressPhase::Initializing,
        checkpoints_total,
        checkpoints_processed,
        None,
        None,
        &counters,
    );

    let result: Result<()> = async {
    let _ = core_extension_host().context("loading Core extension host for `devql ingest`")?;
    let backends = resolve_store_backend_config_for_repo(&cfg.config_root)
        .context("resolving DevQL backend config for `devql ingest`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql ingest").await?;
    let knowledge_context =
        capability_ingest_context_for_ingester(cfg, None, KNOWLEDGE_CAPABILITY_INGESTER_ID)
            .context("resolving knowledge capability ingester owner")?;
    let capability = resolve_embedding_capability_config_for_repo(&cfg.config_root);
    let semantic_clones = capability.semantic_clones.clone();
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
    let active_branch = active_branch_name(&cfg.repo_root);
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

    let mut checkpoints = list_committed(&cfg.repo_root)?;
    checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if max_checkpoints > 0 && checkpoints.len() > max_checkpoints {
        checkpoints.truncate(max_checkpoints);
    }
    checkpoints_total = checkpoints.len();
    emit_progress(
        observer,
        IngestionProgressPhase::Initializing,
        checkpoints_total,
        checkpoints_processed,
        None,
        None,
        &counters,
    );

    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    let mut existing_event_ids = fetch_existing_checkpoint_event_ids(cfg, &backends.events).await?;

    for cp in checkpoints {
        let checkpoint = cp.clone();
        let commit_info = commit_map.get(&cp.checkpoint_id);
        let commit_sha = commit_info
            .map(|info| info.commit_sha.clone())
            .unwrap_or_default();
        let commit_sha_option = (!commit_sha.is_empty()).then_some(commit_sha.clone());
        emit_progress(
            observer,
            IngestionProgressPhase::Extracting,
            checkpoints_total,
            checkpoints_processed,
            Some(cp.checkpoint_id.clone()),
            commit_sha_option.clone(),
            &counters,
        );
        let event_id = deterministic_uuid(&format!(
            "{}|{}|{}|checkpoint_committed",
            cfg.repo.repo_id, cp.checkpoint_id, cp.session_id
        ));

        if !existing_event_ids.contains(&event_id) {
            insert_checkpoint_event(cfg, &backends.events, &cp, &event_id, commit_info).await?;
            existing_event_ids.insert(event_id);
            counters.events_inserted += 1;
        }

        if commit_sha.is_empty() {
            counters.checkpoints_without_commit += 1;
            checkpoints_processed += 1;
            emit_checkpoint_ingested(observer, checkpoint, None);
            emit_progress(
                observer,
                IngestionProgressPhase::Persisting,
                checkpoints_total,
                checkpoints_processed,
                Some(cp.checkpoint_id.clone()),
                None,
                &counters,
            );
            continue;
        }

        upsert_commit_row(
            cfg,
            &relational,
            &cp,
            commit_info.expect("commit_info exists when sha exists"),
        )
        .await?;

        for path in &cp.files_touched {
            let normalized_path = normalize_repo_path(path);
            if normalized_path.is_empty() {
                continue;
            }

            let blob_sha = git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, &normalized_path)
                .or_else(|| git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, path));
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
                    commit_unix: commit_info
                        .expect("commit_info exists when sha exists")
                        .commit_unix,
                    path: &normalized_path,
                    blob_sha: &blob_sha,
                },
                &file_artefact,
            )
            .await?;
            counters.artefacts_upserted += 1;

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
            let semantic_feature_inputs = semantic_clones_pack::build_semantic_feature_inputs(
                &pre_stage_artefacts,
                &pre_stage_dependencies,
                &content,
            );
            let input_hashes = semantic_feature_inputs
                .iter()
                .map(|input| {
                    (
                        input.artefact_id.clone(),
                        semantic::build_semantic_feature_input_hash(input, summary_provider.as_ref()),
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
                    cfg.config_root.clone(),
                    cfg.repo_root.clone(),
                    cfg.repo.repo_id.clone(),
                    active_branch.clone(),
                );
                if semantic_clones.summary_mode == SemanticSummaryMode::Auto {
                    enrichment
                        .enqueue_semantic_summaries(
                            enqueue_target.clone(),
                            semantic_feature_inputs.clone(),
                            input_hashes.clone(),
                            semantic_clones.embedding_mode,
                        )
                        .await?;
                }

                if embedding_outputs_enabled {
                    match semantic_clones.embedding_mode {
                        SemanticCloneEmbeddingMode::Off => {}
                        SemanticCloneEmbeddingMode::Deterministic
                        | SemanticCloneEmbeddingMode::RefreshOnUpgrade => {
                            enrichment
                                .enqueue_symbol_embeddings(
                                    enqueue_target.clone(),
                                    semantic_feature_inputs.clone(),
                                    input_hashes.clone(),
                                    semantic_clones.embedding_mode,
                                )
                                .await?;
                        }
                        SemanticCloneEmbeddingMode::SemanticAwareOnce => {
                            if semantic_clones.summary_mode == SemanticSummaryMode::Off {
                                enrichment
                                    .enqueue_symbol_embeddings(
                                        enqueue_target.clone(),
                                        semantic_feature_inputs.clone(),
                                        input_hashes.clone(),
                                        semantic_clones.embedding_mode,
                                    )
                                    .await?;
                            }
                        }
                    }
                }
            } else if let Some(embedding_provider) = embedding_provider.as_ref() {
                let embedding_stats = upsert_symbol_embedding_rows(
                    &relational,
                    &semantic_feature_inputs,
                    Arc::clone(embedding_provider),
                )
                .await?;
                counters.symbol_embedding_rows_upserted += embedding_stats.upserted;
                counters.symbol_embedding_rows_skipped += embedding_stats.skipped;
            }
            counters.artefacts_upserted += 1;
            counters.semantic_feature_rows_upserted += semantic_feature_stats.upserted;
            counters.semantic_feature_rows_skipped += semantic_feature_stats.skipped;
        }

        let _projected_rows = upsert_checkpoint_file_snapshot_rows(
            cfg,
            &relational,
            &cp,
            &commit_sha,
            commit_info,
        )
        .await?;

        counters.checkpoints_processed += 1;
        checkpoints_processed += 1;
        emit_checkpoint_ingested(observer, checkpoint, commit_sha_option.clone());
        emit_progress(
            observer,
            IngestionProgressPhase::Persisting,
            checkpoints_total,
            checkpoints_processed,
            Some(cp.checkpoint_id.clone()),
            commit_sha_option,
            &counters,
        );
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;
    if let Some(warning) = embedding_warning.as_deref() {
        log::warn!("semantic_clones embeddings degraded; skipping embedding and clone stages: {warning}");
    }

    if enrichment.is_none() && embedding_provider.is_some() {
        let capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
        let clone_ingest = capability_host
            .invoke_ingester_with_relational(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
                json!({}),
                Some(&relational),
            )
            .await
            .with_context(|| {
                format!(
                    "running capability ingester `{SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID}` for `{SEMANTIC_CLONES_CAPABILITY_ID}`"
                )
            })?;
        counters.symbol_clone_edges_upserted += clone_ingest.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize;
        counters.symbol_clone_sources_scored += clone_ingest.payload["symbol_clone_sources_scored"]
            .as_u64()
            .unwrap_or_default() as usize;
    } else if !embedding_outputs_enabled || (enrichment.is_none() && embedding_provider.is_none()) {
        clear_repo_symbol_embedding_rows(&relational, &cfg.repo.repo_id).await?;
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
        checkpoints_total,
        checkpoints_processed,
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
                checkpoints_total,
                checkpoints_processed,
                None,
                None,
                &counters,
            );
            Err(err)
        }
    }
}

fn active_branch_name(repo_root: &Path) -> String {
    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["branch", "--show-current"],
    )
    .ok()
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "main".to_string())
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
    checkpoints_total: usize,
    checkpoints_processed: usize,
    current_checkpoint_id: Option<String>,
    current_commit_sha: Option<String>,
    counters: &IngestionCounters,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.on_progress(IngestionProgressUpdate {
        phase,
        checkpoints_total,
        checkpoints_processed,
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
