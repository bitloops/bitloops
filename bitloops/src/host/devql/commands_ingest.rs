use super::*;

pub async fn run_ingest(cfg: &DevqlConfig, init: bool, max_checkpoints: usize) -> Result<()> {
    let _ = core_extension_host().context("loading Core extension host for `devql ingest`")?;
    let backends = resolve_store_backend_config_for_repo(&cfg.repo_root)
        .context("resolving DevQL backend config for `devql ingest`")?;
    let relational = RelationalStorage::connect(cfg, &backends.relational, "devql ingest").await?;
    let knowledge_context =
        capability_ingest_context_for_ingester(cfg, None, KNOWLEDGE_CAPABILITY_INGESTER_ID)
            .context("resolving knowledge capability ingester owner")?;
    let summary_provider =
        semantic_clones_pack::build_semantic_summary_provider(&semantic_provider_config(cfg))?;
    let embedding_provider = semantic_clones_pack::build_symbol_embedding_provider(
        &embedding_provider_config(cfg),
        Some(&cfg.repo_root),
    )?;
    if init {
        if backends.events.has_clickhouse() {
            init_clickhouse_schema(cfg).await?;
        } else {
            init_duckdb_schema(&backends.events).await?;
        }
        init_relational_schema(cfg, &relational).await?;
    }

    ensure_repository_row(cfg, &relational).await?;

    let mut checkpoints = list_committed(&cfg.repo_root)?;
    checkpoints.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    if max_checkpoints > 0 && checkpoints.len() > max_checkpoints {
        checkpoints.truncate(max_checkpoints);
    }

    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    let mut existing_event_ids = fetch_existing_checkpoint_event_ids(cfg, &backends.events).await?;

    let mut counters = IngestionCounters::default();

    for cp in checkpoints {
        let commit_info = commit_map.get(&cp.checkpoint_id);
        let commit_sha = commit_info
            .map(|info| info.commit_sha.clone())
            .unwrap_or_default();
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
                delete_current_state_for_path(cfg, &relational, &normalized_path).await?;
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
            if let Some(embedding_provider) = embedding_provider.as_ref() {
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

        counters.checkpoints_processed += 1;
    }

    counters.temporary_rows_promoted =
        promote_temporary_current_rows_for_head_commit(cfg, &relational).await?;

    let mut capability_host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    let clone_ingest = capability_host
        .invoke_ingester_with_relational(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_REBUILD_INGESTER_ID,
            json!({}),
            Some(&relational),
        )
        .await
        .with_context(|| {
            format!(
                "running capability ingester `{SEMANTIC_CLONES_REBUILD_INGESTER_ID}` for `{SEMANTIC_CLONES_CAPABILITY_ID}`"
            )
        })?;
    counters.symbol_clone_edges_upserted += clone_ingest.payload["symbol_clone_edges_upserted"]
        .as_u64()
        .unwrap_or_default() as usize;
    counters.symbol_clone_sources_scored += clone_ingest.payload["symbol_clone_sources_scored"]
        .as_u64()
        .unwrap_or_default() as usize;

    println!(
        "DevQL ingest complete: checkpoints_processed={}, events_inserted={}, artefacts_upserted={}, checkpoints_without_commit={}, temporary_rows_promoted={}, semantic_feature_rows_upserted={}, semantic_feature_rows_skipped={}, symbol_embedding_rows_upserted={}, symbol_embedding_rows_skipped={}, symbol_clone_edges_upserted={}, symbol_clone_sources_scored={}",
        counters.checkpoints_processed,
        counters.events_inserted,
        counters.artefacts_upserted,
        counters.checkpoints_without_commit,
        counters.temporary_rows_promoted,
        counters.semantic_feature_rows_upserted,
        counters.semantic_feature_rows_skipped,
        counters.symbol_embedding_rows_upserted,
        counters.symbol_embedding_rows_skipped,
        counters.symbol_clone_edges_upserted,
        counters.symbol_clone_sources_scored
    );
    Ok(())
}

pub(crate) async fn promote_temporary_current_rows_for_head_commit(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<usize> {
    let head_sha = run_git(&cfg.repo_root, &["rev-parse", "HEAD"]).unwrap_or_default();
    if head_sha.is_empty() {
        return Ok(0);
    }
    let head_unix = run_git(&cfg.repo_root, &["show", "-s", "--format=%ct", &head_sha])
        .ok()
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .unwrap_or_default();
    let updated_at_sql = revision_timestamp_sql(relational, head_unix);
    let branch = active_branch_name(&cfg.repo_root);

    let sql = format!(
        "SELECT path, blob_sha FROM artefacts_current \
		WHERE repo_id = '{}' AND branch = '{}' AND canonical_kind = 'file' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&branch),
    );
    let rows = relational.query_rows(&sql).await?;
    let mut promoted = 0usize;

    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(blob_sha) = row.get("blob_sha").and_then(Value::as_str) else {
            continue;
        };
        let Some(head_blob_sha) = git_blob_sha_at_commit(&cfg.repo_root, &head_sha, path) else {
            continue;
        };
        if head_blob_sha != blob_sha {
            continue;
        }

        upsert_file_state_row(
            &cfg.repo.repo_id,
            relational,
            &head_sha,
            path,
            &head_blob_sha,
        )
        .await?;

        let sql_artefacts = format!(
            "UPDATE artefacts_current \
		SET commit_sha = '{}', revision_kind = 'commit', revision_id = '{}', temp_checkpoint_id = NULL, blob_sha = '{}', updated_at = {} \
		WHERE repo_id = '{}' AND branch = '{}' AND path = '{}' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
            esc_pg(&head_sha),
            esc_pg(&head_sha),
            esc_pg(&head_blob_sha),
            updated_at_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&branch),
            esc_pg(path),
        );
        relational.exec(&sql_artefacts).await?;

        let sql_edges = format!(
            "UPDATE artefact_edges_current \
		SET commit_sha = '{}', revision_kind = 'commit', revision_id = '{}', temp_checkpoint_id = NULL, blob_sha = '{}', updated_at = {} \
		WHERE repo_id = '{}' AND branch = '{}' AND path = '{}' AND (revision_kind = 'temporary' OR revision_id LIKE 'temp:%')",
            esc_pg(&head_sha),
            esc_pg(&head_sha),
            esc_pg(&head_blob_sha),
            updated_at_sql,
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&branch),
            esc_pg(path),
        );
        relational.exec(&sql_edges).await?;

        promoted += 1;
    }

    Ok(promoted)
}
