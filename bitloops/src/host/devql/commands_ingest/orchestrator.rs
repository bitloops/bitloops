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

pub(crate) async fn select_ingest_backfill_commits_for_head(
    cfg: &DevqlConfig,
    head_sha: &str,
    backfill_window: usize,
) -> Result<Vec<String>> {
    let head_sha = head_sha.trim();
    if head_sha.is_empty() || backfill_window == 0 {
        return Ok(Vec::new());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-merge ingest commit selection")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "post-merge ingest commit selection",
    )
    .await?;
    ensure_repository_row(cfg, &relational).await?;
    select_recent_branch_commit_backfill_window(
        &cfg.repo_root,
        &relational,
        &cfg.repo.repo_id,
        head_sha,
        backfill_window,
    )
    .await
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
    counters.artefacts_upserted +=
        mirror_completed_current_artefacts_for_head(cfg, &relational, &head_sha).await?;
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
        let history_needs_artefact_metadata_repair = if existing_ledger
            .as_ref()
            .is_some_and(commit_is_fully_ingested)
        {
            commit_history_needs_artefact_metadata_repair(
                &cfg.repo_root,
                &relational,
                &cfg.repo.repo_id,
                &commit_sha,
            )
            .await?
        } else {
            false
        };
        if existing_ledger.as_ref().is_some_and(commit_is_fully_ingested)
            && !history_needs_artefact_metadata_repair
        {
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
        let ledger_history_completed = existing_ledger
            .as_ref()
            .map(|entry| entry.history_status == "completed")
            .unwrap_or(false);

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
        if history_needs_artefact_metadata_repair {
            history_completed = false;
        }

        let commit_result: Result<()> = async {
            if !history_completed {
                upsert_commit_metadata_row(cfg, &relational, &commit_info).await?;
                let raw_diff = git_show_hunk_diff(&cfg.repo_root, &commit_sha)?;
                let mut parsed_hunks =
                    parse_commit_hunks_from_git_show(&cfg.repo.repo_id, &commit_sha, &raw_diff)
                        .with_context(|| {
                            format!("parsing hunk diff for commit {commit_sha}")
                        })?;
                filter_commit_hunks_by_exclusions(&mut parsed_hunks, &exclusion_matcher);
                counters.artefacts_upserted += append_changed_after_side_commit_artefacts(
                    cfg,
                    &relational,
                    &commit_sha,
                    &commit_info,
                    &parsed_hunks,
                    &exclusion_matcher,
                    &parser_version,
                    &extractor_version,
                )
                .await?;

                if !ledger_history_completed {
                    mark_commit_history_completed(
                        &relational,
                        &cfg.repo.repo_id,
                        &commit_sha,
                        checkpoint_id.as_deref(),
                    )
                    .await?;
                }
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

    let _ = mirror_completed_current_artefacts_for_head(cfg, &relational, &head_sha).await?;
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

#[derive(Debug, Clone, Copy)]
struct ChangedLineRange {
    start: i32,
    end: i32,
}

async fn append_changed_after_side_commit_artefacts(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_sha: &str,
    commit_info: &CheckpointCommitInfo,
    parsed_hunks: &ParsedCommitHunks,
    exclusion_matcher: &RepoExclusionMatcher,
    parser_version: &str,
    extractor_version: &str,
) -> Result<usize> {
    if parsed_hunks.file_deltas.is_empty() {
        return Ok(0);
    }

    let changed_ranges_by_delta = after_side_changed_line_ranges_by_delta(parsed_hunks);
    if changed_ranges_by_delta.is_empty() {
        return Ok(0);
    }

    let tracked_paths = tracked_paths_at_revision(&cfg.repo_root, commit_sha)
        .with_context(|| format!("listing tracked files for commit {commit_sha}"))?;
    let classifier = ProjectAwareClassifier::discover_for_revision(
        &cfg.repo_root,
        commit_sha,
        tracked_paths,
        parser_version,
        extractor_version,
    )
    .with_context(|| format!("building project-aware classifier for commit {commit_sha}"))?;

    let mut seen_revisions = HashSet::new();
    let mut sql_batch = Vec::new();
    let mut artefacts_appended = 0usize;
    for delta in &parsed_hunks.file_deltas {
        if delta.is_binary {
            continue;
        }
        let Some(changed_ranges) = changed_ranges_by_delta.get(delta.delta_id.as_str()) else {
            continue;
        };
        let Some((path, blob_sha)) = after_side_changed_file_revision_for_delta(delta) else {
            continue;
        };
        if !seen_revisions.insert((path.to_string(), blob_sha.to_string())) {
            continue;
        }

        let excluded_by_policy = exclusion_matcher.excludes_repo_relative_path(path);
        let classification = classifier
            .classify_repo_relative_path(path, excluded_by_policy)
            .with_context(|| {
                format!("classifying hunk ingest artefact path `{path}` at commit {commit_sha}")
            })?;
        if classification.analysis_mode == AnalysisMode::Excluded
            || !classification.should_extract()
        {
            continue;
        }

        let blob_content = git_blob_decoded_content(&cfg.repo_root, blob_sha).ok_or_else(|| {
            anyhow!(
                "failed to decode blob content for hunk ingest artefact path `{}` at commit {} (blob {})",
                path,
                commit_sha,
                blob_sha
            )
        })?;
        if classification.analysis_mode == AnalysisMode::Text {
            let Some(content) = blob_content.text.as_deref() else {
                continue;
            };
            if !plain_text_content_is_allowed(content) {
                continue;
            }
        }

        let (file_artefact, file_record) = build_file_artefact_metadata_record(
            cfg,
            path,
            blob_sha,
            &classification.language,
            &classification.extraction_fingerprint,
            &blob_content,
        );
        sql_batch.push(build_insert_historical_artefact_sql(
            cfg,
            relational,
            &file_artefact.language,
            &file_artefact.extraction_fingerprint,
            &file_record,
        ));
        sql_batch.push(build_insert_commit_artefact_sql(
            relational,
            &cfg.repo.repo_id,
            commit_sha,
            path,
            blob_sha,
            &file_record,
        ));
        artefacts_appended += 1;

        if classification.analysis_mode == AnalysisMode::Text || blob_content.decode_degraded {
            continue;
        }
        let source_content = blob_content.text.as_deref().unwrap_or_default();
        artefacts_appended += append_overlapping_language_artefact_metadata_sql(
            cfg,
            relational,
            &FileRevision {
                commit_sha,
                revision: TemporalRevisionRef {
                    kind: TemporalRevisionKind::Commit,
                    id: commit_sha,
                    temp_checkpoint_id: None,
                },
                commit_unix: commit_info.commit_unix,
                path,
                blob_sha,
            },
            &file_artefact,
            source_content,
            changed_ranges,
            &mut sql_batch,
        )?;
    }

    if !sql_batch.is_empty() {
        relational
            .exec_batch_transactional_for_role(RelationalStorageRole::SharedRelational, &sql_batch)
            .await
            .context("appending changed after-side commit artefacts")?;
    }

    Ok(artefacts_appended)
}

fn after_side_changed_line_ranges_by_delta(
    parsed_hunks: &ParsedCommitHunks,
) -> HashMap<&str, Vec<ChangedLineRange>> {
    let mut ranges_by_delta = HashMap::new();
    for hunk in &parsed_hunks.hunks {
        let Some(range) = after_side_changed_line_range(hunk) else {
            continue;
        };
        ranges_by_delta
            .entry(hunk.delta_id.as_str())
            .or_insert_with(Vec::new)
            .push(range);
    }
    ranges_by_delta
}

fn after_side_changed_line_range(hunk: &CommitHunk) -> Option<ChangedLineRange> {
    let start = hunk.added_lines.iter().map(|line| line.line_number).min()?;
    let end = hunk
        .added_lines
        .iter()
        .map(|line| line.line_number)
        .max()
        .unwrap_or(start);
    Some(ChangedLineRange { start, end })
}

fn build_file_artefact_metadata_record(
    cfg: &DevqlConfig,
    path: &str,
    blob_sha: &str,
    language: &str,
    extraction_fingerprint: &str,
    blob_content: &DecodedFileContent,
) -> (FileArtefactRow, PersistedArtefactRecord) {
    let symbol_id = file_symbol_id(path);
    let artefact_id = revision_artefact_id(&cfg.repo.repo_id, blob_sha, &symbol_id);
    let line_count = blob_content.line_count().max(1);
    let byte_count = blob_content.byte_count().max(0);
    let file_docstring = blob_content
        .text
        .as_deref()
        .and_then(|content| extract_file_docstring_for_language_pack(path, language, content));
    let file_artefact = FileArtefactRow {
        artefact_id,
        symbol_id,
        language: language.to_string(),
        extraction_fingerprint: extraction_fingerprint.to_string(),
        end_line: line_count,
        end_byte: byte_count,
    };
    let record = build_file_current_record(path, blob_sha, &file_artefact, file_docstring);
    (file_artefact, record)
}

fn append_overlapping_language_artefact_metadata_sql(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    rev: &FileRevision<'_>,
    file_artefact: &FileArtefactRow,
    source_content: &str,
    changed_ranges: &[ChangedLineRange],
    sql_batch: &mut Vec<String>,
) -> Result<usize> {
    let Some((_context, pack_id)) = language_pack_context_for_language(
        cfg,
        Some(rev.commit_sha),
        &file_artefact.language,
        Some(rev.path),
    )
    .with_context(|| {
        format!(
            "resolving language pack owner for `{}`",
            file_artefact.language
        )
    })?
    else {
        return Ok(0);
    };

    let registry = language_adapter_registry()?;
    let items = registry.extract_artefacts(pack_id, source_content, rev.path)?;
    let symbol_records = build_symbol_records(
        cfg,
        rev.path,
        rev.blob_sha,
        file_artefact,
        &items,
        source_content,
    );
    let mut appended = 0usize;
    for record in symbol_records
        .iter()
        .filter(|record| artefact_overlaps_changed_ranges(record, changed_ranges))
    {
        sql_batch.push(build_insert_historical_artefact_sql(
            cfg,
            relational,
            &file_artefact.language,
            &file_artefact.extraction_fingerprint,
            record,
        ));
        sql_batch.push(build_insert_commit_artefact_sql(
            relational,
            &cfg.repo.repo_id,
            rev.commit_sha,
            rev.path,
            rev.blob_sha,
            record,
        ));
        appended += 1;
    }
    Ok(appended)
}

fn artefact_overlaps_changed_ranges(
    record: &PersistedArtefactRecord,
    changed_ranges: &[ChangedLineRange],
) -> bool {
    changed_ranges
        .iter()
        .any(|range| record.start_line <= range.end && record.end_line >= range.start)
}

fn build_insert_historical_artefact_sql(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    language: &str,
    extraction_fingerprint: &str,
    record: &PersistedArtefactRecord,
) -> String {
    let shared_dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let canonical_kind_sql = sql_nullable_text(record.canonical_kind.as_deref());
    let signature_sql = sql_nullable_text(record.signature.as_deref());
    let modifiers_sql = sql_json_text_array_for_dialect(shared_dialect, &record.modifiers);
    let docstring_sql = sql_nullable_text(record.docstring.as_deref());
    format!(
        "INSERT INTO artefacts (artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind, language_kind, symbol_fqn, signature, modifiers, docstring, content_hash) \
VALUES ('{}', '{}', '{}', '{}', '{}', {}, '{}', '{}', {}, {}, {}, '{}') \
ON CONFLICT (artefact_id) DO NOTHING",
        esc_pg(&record.artefact_id),
        esc_pg(&record.symbol_id),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(language),
        esc_pg(extraction_fingerprint),
        canonical_kind_sql,
        esc_pg(&record.language_kind),
        esc_pg(&record.symbol_fqn),
        signature_sql,
        modifiers_sql,
        docstring_sql,
        esc_pg(&record.content_hash),
    )
}

fn build_insert_commit_artefact_sql(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    path: &str,
    blob_sha: &str,
    record: &PersistedArtefactRecord,
) -> String {
    let parent_artefact_id = sql_nullable_text(record.parent_artefact_id.as_deref());
    let now_sql =
        sql_now_for_dialect(relational.dialect_for_role(RelationalStorageRole::SharedRelational));
    format!(
        "INSERT INTO commit_artefacts (
            repo_id, commit_sha, artefact_id, path, blob_sha, parent_artefact_id,
            start_line, end_line, start_byte, end_byte, created_at
         ) VALUES (
            '{repo_id}', '{commit_sha}', '{artefact_id}', '{path}', '{blob_sha}', {parent_artefact_id},
            {start_line}, {end_line}, {start_byte}, {end_byte}, {now_sql}
         )
         ON CONFLICT (repo_id, commit_sha, artefact_id) DO NOTHING",
        repo_id = esc_pg(repo_id),
        commit_sha = esc_pg(commit_sha),
        artefact_id = esc_pg(&record.artefact_id),
        path = esc_pg(path),
        blob_sha = esc_pg(blob_sha),
        parent_artefact_id = parent_artefact_id,
        start_line = record.start_line,
        end_line = record.end_line,
        start_byte = record.start_byte,
        end_byte = record.end_byte,
        now_sql = now_sql,
    )
}

async fn mirror_completed_current_artefacts_for_head(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    head_sha: &str,
) -> Result<usize> {
    let head_sha = head_sha.trim();
    if head_sha.is_empty() {
        return Ok(0);
    }

    if !repo_sync_state_matches_completed_head(relational, &cfg.repo.repo_id, head_sha).await? {
        return Ok(0);
    }

    let current_rows = current_artefact_rows_for_repo_count(relational, &cfg.repo.repo_id).await?;
    if current_rows == 0 {
        return Ok(0);
    }

    if relational.has_remote_shared_relational_authority() {
        mirror_current_artefacts_to_remote_shared(relational, &cfg.repo.repo_id).await?;
    } else {
        relational
            .exec_for_role(
                RelationalStorageRole::SharedRelational,
                &build_current_artefact_mirror_insert_select_sql(&cfg.repo.repo_id),
            )
            .await
            .context("mirroring current artefacts into canonical artefacts")?;
    }

    Ok(current_rows)
}

async fn repo_sync_state_matches_completed_head(
    relational: &RelationalStorage,
    repo_id: &str,
    head_sha: &str,
) -> Result<bool> {
    let rows = relational
        .query_rows(&format!(
            "SELECT last_sync_status, head_commit_sha \
             FROM repo_sync_state \
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("loading repo sync state before ingest current artefact mirror")?;
    let Some(row) = rows.first() else {
        return Ok(false);
    };
    let status = row
        .get("last_sync_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let synced_head = row
        .get("head_commit_sha")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(status == "completed" && synced_head == head_sha)
}

async fn current_artefact_rows_for_repo_count(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<usize> {
    let rows = relational
        .query_rows(&format!(
            "SELECT COUNT(*) AS row_count \
             FROM artefacts_current \
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("counting current artefacts before ingest mirror")?;
    let raw = rows
        .first()
        .and_then(|row| row.get("row_count"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .unwrap_or(0);
    usize::try_from(raw).context("current artefact row count exceeds usize")
}

fn build_current_artefact_mirror_insert_select_sql(repo_id: &str) -> String {
    let repo_id = esc_pg(repo_id);
    format!(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
         )
         SELECT
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_id
         FROM artefacts_current
         WHERE repo_id = '{repo_id}'
         ON CONFLICT (artefact_id) DO UPDATE SET
            symbol_id = EXCLUDED.symbol_id,
            repo_id = EXCLUDED.repo_id,
            language = EXCLUDED.language,
            extraction_fingerprint = EXCLUDED.extraction_fingerprint,
            canonical_kind = EXCLUDED.canonical_kind,
            language_kind = EXCLUDED.language_kind,
            symbol_fqn = EXCLUDED.symbol_fqn,
            signature = EXCLUDED.signature,
            modifiers = EXCLUDED.modifiers,
            docstring = EXCLUDED.docstring,
            content_hash = EXCLUDED.content_hash",
    )
}

async fn mirror_current_artefacts_to_remote_shared(
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<()> {
    let rows = relational
        .query_rows(&format!(
            "SELECT
                artefact_id,
                symbol_id,
                repo_id,
                language,
                extraction_fingerprint,
                canonical_kind,
                language_kind,
                symbol_fqn,
                signature,
                modifiers,
                docstring,
                content_id AS content_hash
             FROM artefacts_current
             WHERE repo_id = '{}'",
            esc_pg(repo_id),
        ))
        .await
        .context("loading current artefacts for remote canonical mirror")?;
    if rows.is_empty() {
        return Ok(());
    }

    let dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let statements = rows
        .iter()
        .map(|row| build_current_artefact_mirror_upsert_sql(dialect, row))
        .collect::<Result<Vec<_>>>()?;
    relational
        .exec_batch_transactional_for_role(RelationalStorageRole::SharedRelational, &statements)
        .await
        .context("mirroring current artefacts into remote canonical artefacts")
}

fn build_current_artefact_mirror_upsert_sql(
    dialect: RelationalDialect,
    row: &serde_json::Value,
) -> Result<String> {
    Ok(format!(
        "INSERT INTO artefacts (
            artefact_id, symbol_id, repo_id, language, extraction_fingerprint, canonical_kind,
            language_kind, symbol_fqn, signature, modifiers, docstring, content_hash
         ) VALUES (
            '{artefact_id}', {symbol_id}, '{repo_id}', '{language}', {extraction_fingerprint},
            {canonical_kind}, {language_kind}, {symbol_fqn}, {signature}, {modifiers},
            {docstring}, '{content_hash}'
         )
         ON CONFLICT (artefact_id) DO UPDATE SET
            symbol_id = EXCLUDED.symbol_id,
            repo_id = EXCLUDED.repo_id,
            language = EXCLUDED.language,
            extraction_fingerprint = EXCLUDED.extraction_fingerprint,
            canonical_kind = EXCLUDED.canonical_kind,
            language_kind = EXCLUDED.language_kind,
            symbol_fqn = EXCLUDED.symbol_fqn,
            signature = EXCLUDED.signature,
            modifiers = EXCLUDED.modifiers,
            docstring = EXCLUDED.docstring,
            content_hash = EXCLUDED.content_hash",
        artefact_id = esc_pg(row_required_str(row, "artefact_id")?),
        symbol_id = sql_nullable_text(row_optional_str(row, "symbol_id")),
        repo_id = esc_pg(row_required_str(row, "repo_id")?),
        language = esc_pg(row_required_str(row, "language")?),
        extraction_fingerprint = sql_nullable_text(row_optional_str(row, "extraction_fingerprint")),
        canonical_kind = sql_nullable_text(row_optional_str(row, "canonical_kind")),
        language_kind = sql_nullable_text(row_optional_str(row, "language_kind")),
        symbol_fqn = sql_nullable_text(row_optional_str(row, "symbol_fqn")),
        signature = sql_nullable_text(row_optional_str(row, "signature")),
        modifiers = sql_json_value_for_dialect(dialect, &row_json_value(row, "modifiers", "[]")?),
        docstring = sql_nullable_text(row_optional_str(row, "docstring")),
        content_hash = esc_pg(row_required_str(row, "content_hash")?),
    ))
}

fn row_required_str<'a>(row: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("current artefact mirror row is missing string column `{key}`"))
}

fn row_optional_str<'a>(row: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

fn row_json_value(row: &serde_json::Value, key: &str, default: &str) -> Result<serde_json::Value> {
    match row.get(key) {
        Some(serde_json::Value::String(raw)) if !raw.is_empty() => serde_json::from_str(raw)
            .with_context(|| format!("parsing current artefact mirror JSON column `{key}`")),
        Some(serde_json::Value::Array(_)) | Some(serde_json::Value::Object(_)) => {
            Ok(row.get(key).cloned().unwrap_or(serde_json::Value::Null))
        }
        Some(serde_json::Value::Null) | None => serde_json::from_str(default).with_context(|| {
            format!("parsing default JSON for current artefact mirror column `{key}`")
        }),
        Some(other) => serde_json::from_str(&other.to_string())
            .with_context(|| format!("parsing current artefact mirror JSON column `{key}`")),
    }
}

fn sql_nullable_text(value: Option<&str>) -> String {
    value
        .map(|text| format!("'{}'", esc_pg(text)))
        .unwrap_or_else(|| "NULL".to_string())
}

fn after_side_changed_file_revision_for_delta(delta: &CommitFileDelta) -> Option<(&str, &str)> {
    delta
        .path_after
        .as_deref()
        .zip(delta.new_blob_sha.as_deref())
}
