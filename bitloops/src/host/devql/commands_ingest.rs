use super::*;
use crate::capability_packs::semantic_clones::embeddings;
use crate::capability_packs::semantic_clones::features as semantic;
use crate::capability_packs::semantic_clones::ingesters::{
    EmbeddingRefreshMode, SemanticFeaturesRefreshPayload, SemanticFeaturesRefreshScope,
    SemanticSummaryRefreshMode, SymbolEmbeddingsRefreshPayload, SymbolEmbeddingsRefreshScope,
};
use crate::capability_packs::semantic_clones::runtime_config::{
    EmbeddingProviderMode, SummaryProviderMode, embeddings_enabled, resolve_embedding_provider,
    resolve_semantic_clones_config, resolve_summary_provider,
};
use crate::capability_packs::semantic_clones::workplane::{
    enqueue_embedding_jobs, enqueue_summary_refresh_jobs, resolve_effective_mailbox_intent,
};
use crate::capability_packs::semantic_clones::{
    RepoEmbeddingSyncAction, clear_repo_active_embedding_setup, clear_repo_symbol_embedding_rows,
    determine_repo_embedding_sync_action, load_active_embedding_setup,
    load_current_repo_embedding_states, load_semantic_feature_inputs_for_current_repo,
    persist_active_embedding_setup,
};
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
    let capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    let knowledge_context =
        capability_ingest_context_for_ingester(cfg, None, KNOWLEDGE_CAPABILITY_INGESTER_ID)
            .context("resolving knowledge capability ingester owner")?;
    let semantic_clones = resolve_semantic_clones_config(
        &capability_host.config_view(SEMANTIC_CLONES_CAPABILITY_ID),
    );
    let semantic_clones_workplane = if enrichment.is_some() {
        Some(capability_host.build_workplane_gateway(SEMANTIC_CLONES_CAPABILITY_ID)?)
    } else {
        None
    };
    let semantic_clones_workplane_intent = match semantic_clones_workplane.as_ref() {
        Some(workplane) => Some(resolve_effective_mailbox_intent(
            workplane.as_ref(),
            &semantic_clones,
        )?),
        None => None,
    };
    let semantic_inference = capability_host.inference_for_capability(SEMANTIC_CLONES_CAPABILITY_ID);
    let preferred_representation_kind =
        crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code;
    let embedding_outputs_enabled = embeddings_enabled(&semantic_clones);
    let mut embedding_warning = None;
    let mut direct_embedding_provider_available = false;
    let mut resolved_embedding_setup = None;
    let direct_embedding_sync_action = if enrichment.is_none() && embedding_outputs_enabled {
        let selection = resolve_embedding_provider(
            &semantic_clones,
            &semantic_inference,
            preferred_representation_kind,
            EmbeddingProviderMode::ConfiguredDegrade,
        )?;
        if selection.degraded_reason.is_some() {
            embedding_warning = selection.degraded_reason.clone();
        }
        if let Some(embedding_provider) = selection.provider.as_ref() {
            let setup = embeddings::resolve_embedding_setup(embedding_provider.as_ref())?;
            let action = determine_repo_embedding_sync_action(
                &relational,
                &cfg.repo.repo_id,
                preferred_representation_kind,
                &setup,
            )
            .await?;
            resolved_embedding_setup = Some(setup);
            direct_embedding_provider_available = true;
            Some(action)
        } else {
            None
        }
    } else {
        None
    };

    ensure_repository_row(cfg, &relational).await?;

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
                        semantic::build_semantic_feature_inputs_from_artefacts_with_dependencies(
                            &pre_stage_artefacts,
                            &pre_stage_dependencies,
                            &content,
                        );
                    let (semantic_feature_stats, input_hashes, _enriched) =
                        run_semantic_features_refresh(
                            &capability_host,
                            &relational,
                            SemanticFeaturesRefreshPayload {
                                scope: SemanticFeaturesRefreshScope::Historical,
                                path: None,
                                content_id: None,
                                inputs: semantic_feature_inputs.clone(),
                                mode: if enrichment.is_some() {
                                    SemanticSummaryRefreshMode::DeterministicOnly
                                } else {
                                    SemanticSummaryRefreshMode::ConfiguredDegrade
                                },
                            },
                        )
                    .await
                    .with_context(|| {
                        format!(
                            "running capability ingester `{}` owned by `{}`",
                            knowledge_context.ingester_id, knowledge_context.capability_pack_id
                        )
                    })?;
                    if let Some(workplane) = semantic_clones_workplane.as_ref() {
                        if let Some(intent) = semantic_clones_workplane_intent.as_ref() {
                            enqueue_summary_refresh_jobs(
                                workplane.as_ref(),
                                &semantic_feature_inputs,
                                intent,
                            )?;
                            enqueue_embedding_jobs(
                                workplane.as_ref(),
                                &semantic_feature_inputs,
                                intent,
                            )?;
                        }
                    } else if direct_embedding_provider_available {
                        let code_refresh = run_symbol_embeddings_refresh(
                            &capability_host,
                            &relational,
                            SymbolEmbeddingsRefreshPayload {
                                scope: SymbolEmbeddingsRefreshScope::Historical,
                                path: None,
                                content_id: None,
                                inputs: semantic_feature_inputs.clone(),
                                expected_input_hashes: input_hashes.clone(),
                                representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                                mode: EmbeddingRefreshMode::ConfiguredDegrade,
                                manage_active_state: false,
                            },
                        )
                        .await?;
                        apply_symbol_embedding_refresh_counts(&mut counters, &code_refresh);

                        let summary_refresh = run_symbol_embeddings_refresh(
                            &capability_host,
                            &relational,
                            SymbolEmbeddingsRefreshPayload {
                                scope: SymbolEmbeddingsRefreshScope::Historical,
                                path: None,
                                content_id: None,
                                inputs: semantic_feature_inputs.clone(),
                                expected_input_hashes: input_hashes.clone(),
                                representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                                mode: EmbeddingRefreshMode::ConfiguredDegrade,
                                manage_active_state: false,
                            },
                        )
                        .await?;
                        apply_symbol_embedding_refresh_counts(&mut counters, &summary_refresh);
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

    if let Some(workplane) = semantic_clones_workplane.as_ref()
        && semantic_clones_workplane_intent
            .as_ref()
            .is_some_and(|intent| intent.has_any_embedding_intent())
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
            let bootstrap_summary_provider = resolve_summary_provider(
                &semantic_clones,
                &semantic_inference,
                SummaryProviderMode::DeterministicOnly,
            )?;
            let _bootstrap_input_hashes = bootstrap_inputs
                .iter()
                .map(|input| {
                    (
                        input.artefact_id.clone(),
                        semantic::build_semantic_feature_input_hash(
                            input,
                            bootstrap_summary_provider.provider.as_ref(),
                        ),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            if let Some(intent) = semantic_clones_workplane_intent.as_ref() {
                enqueue_summary_refresh_jobs(workplane.as_ref(), &bootstrap_inputs, intent)?;
                enqueue_embedding_jobs(workplane.as_ref(), &bootstrap_inputs, intent)?;
            }
        }
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;
    if let Some(warning) = embedding_warning.as_deref() {
        log::warn!("semantic_clones embeddings degraded; skipping embedding and clone stages: {warning}");
    }

    if enrichment.is_none() && direct_embedding_provider_available {
        match direct_embedding_sync_action.unwrap_or(RepoEmbeddingSyncAction::Incremental) {
            RepoEmbeddingSyncAction::RefreshCurrentRepo => {
                clear_repo_symbol_embedding_rows(&relational, &cfg.repo.repo_id).await?;
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
                if !current_inputs.is_empty() {
                    let code_refresh = run_symbol_embeddings_refresh(
                        &capability_host,
                        &relational,
                        SymbolEmbeddingsRefreshPayload {
                            scope: SymbolEmbeddingsRefreshScope::Historical,
                            path: None,
                            content_id: None,
                            inputs: current_inputs.clone(),
                            expected_input_hashes: Default::default(),
                            representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
                            mode: EmbeddingRefreshMode::ConfiguredDegrade,
                            manage_active_state: true,
                        },
                    )
                    .await?;
                    apply_symbol_embedding_refresh_counts(&mut counters, &code_refresh);
                    if code_refresh.clone_rebuild_recommended {
                        let clone_ingest =
                            rebuild_active_clone_edges(&capability_host, &relational).await?;
                        counters.symbol_clone_edges_upserted += clone_ingest.0;
                        counters.symbol_clone_sources_scored += clone_ingest.1;
                    }

                    let summary_refresh = run_symbol_embeddings_refresh(
                        &capability_host,
                        &relational,
                        SymbolEmbeddingsRefreshPayload {
                            scope: SymbolEmbeddingsRefreshScope::Historical,
                            path: None,
                            content_id: None,
                            inputs: current_inputs,
                            expected_input_hashes: Default::default(),
                            representation_kind: crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Summary,
                            mode: EmbeddingRefreshMode::ConfiguredDegrade,
                            manage_active_state: true,
                        },
                    )
                    .await?;
                    apply_symbol_embedding_refresh_counts(&mut counters, &summary_refresh);
                }
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
                let clone_ingest =
                    rebuild_active_clone_edges(&capability_host, &relational).await?;
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
    } else if !embedding_outputs_enabled
        || (enrichment.is_none() && !direct_embedding_provider_available)
    {
        clear_repo_symbol_embedding_rows(&relational, &cfg.repo.repo_id).await?;
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
        Some(
            crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind::Code,
        ),
    )
    .await?;
    Ok(states.into_iter().find(|state| state.setup == *setup))
}

async fn run_semantic_features_refresh(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    payload: SemanticFeaturesRefreshPayload,
) -> Result<(
    semantic::SemanticFeatureIngestionStats,
    std::collections::BTreeMap<String, String>,
    bool,
)> {
    let result = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
            serde_json::to_value(&payload)?,
            Some(relational),
        )
        .await?;
    Ok((
        semantic::SemanticFeatureIngestionStats {
            upserted: result.payload["semantic_feature_rows_upserted"]
                .as_u64()
                .unwrap_or_default() as usize,
            skipped: result.payload["semantic_feature_rows_skipped"]
                .as_u64()
                .unwrap_or_default() as usize,
        },
        parse_string_map(&result.payload["input_hashes"]),
        result.payload["produced_enriched_semantics"]
            .as_bool()
            .unwrap_or(false),
    ))
}

#[derive(Debug, Clone, Default)]
struct SymbolEmbeddingsRefreshOutcome {
    semantic_feature_rows_upserted: usize,
    semantic_feature_rows_skipped: usize,
    symbol_embedding_rows_upserted: usize,
    symbol_embedding_rows_skipped: usize,
    clone_rebuild_recommended: bool,
    symbol_clone_edges_upserted: usize,
    symbol_clone_sources_scored: usize,
}

async fn run_symbol_embeddings_refresh(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
    payload: SymbolEmbeddingsRefreshPayload,
) -> Result<SymbolEmbeddingsRefreshOutcome> {
    let result = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
            serde_json::to_value(&payload)?,
            Some(relational),
        )
        .await?;
    Ok(SymbolEmbeddingsRefreshOutcome {
        semantic_feature_rows_upserted: result.payload["semantic_feature_rows_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        semantic_feature_rows_skipped: result.payload["semantic_feature_rows_skipped"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_embedding_rows_upserted: result.payload["symbol_embedding_rows_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_embedding_rows_skipped: result.payload["symbol_embedding_rows_skipped"]
            .as_u64()
            .unwrap_or_default() as usize,
        clone_rebuild_recommended: result.payload["clone_rebuild_recommended"]
            .as_bool()
            .unwrap_or(false),
        symbol_clone_edges_upserted: result.payload["symbol_clone_edges_upserted"]
            .as_u64()
            .unwrap_or_default() as usize,
        symbol_clone_sources_scored: result.payload["symbol_clone_sources_scored"]
            .as_u64()
            .unwrap_or_default() as usize,
    })
}

fn apply_symbol_embedding_refresh_counts(
    counters: &mut IngestionCounters,
    outcome: &SymbolEmbeddingsRefreshOutcome,
) {
    counters.semantic_feature_rows_upserted += outcome.semantic_feature_rows_upserted;
    counters.semantic_feature_rows_skipped += outcome.semantic_feature_rows_skipped;
    counters.symbol_embedding_rows_upserted += outcome.symbol_embedding_rows_upserted;
    counters.symbol_embedding_rows_skipped += outcome.symbol_embedding_rows_skipped;
    counters.symbol_clone_edges_upserted += outcome.symbol_clone_edges_upserted;
    counters.symbol_clone_sources_scored += outcome.symbol_clone_sources_scored;
}

fn parse_string_map(value: &serde_json::Value) -> std::collections::BTreeMap<String, String> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn rebuild_active_clone_edges(
    capability_host: &crate::host::capability_host::DevqlCapabilityHost,
    relational: &RelationalStorage,
) -> Result<(usize, usize)> {
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
