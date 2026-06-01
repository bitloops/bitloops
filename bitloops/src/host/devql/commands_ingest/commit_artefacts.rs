use super::shared::tracked_paths_at_revision;
use super::*;

#[derive(Debug, Clone, Copy)]
struct ChangedLineRange {
    start: i32,
    end: i32,
}

pub(super) struct CommitArtefactAppendContext<'a> {
    pub(super) cfg: &'a DevqlConfig,
    pub(super) relational: &'a RelationalStorage,
    pub(super) exclusion_matcher: &'a RepoExclusionMatcher,
    pub(super) parser_version: &'a str,
    pub(super) extractor_version: &'a str,
}

pub(super) async fn append_changed_after_side_commit_artefacts(
    ctx: &CommitArtefactAppendContext<'_>,
    commit_sha: &str,
    commit_info: &CheckpointCommitInfo,
    parsed_hunks: &ParsedCommitHunks,
) -> Result<usize> {
    if parsed_hunks.file_deltas.is_empty() {
        return Ok(0);
    }

    let changed_ranges_by_delta = after_side_changed_line_ranges_by_delta(parsed_hunks);
    if changed_ranges_by_delta.is_empty() {
        return Ok(0);
    }

    let tracked_paths = tracked_paths_at_revision(&ctx.cfg.repo_root, commit_sha)
        .with_context(|| format!("listing tracked files for commit {commit_sha}"))?;
    let classifier = ProjectAwareClassifier::discover_for_revision(
        &ctx.cfg.repo_root,
        commit_sha,
        tracked_paths,
        ctx.parser_version,
        ctx.extractor_version,
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

        let excluded_by_policy = ctx.exclusion_matcher.excludes_repo_relative_path(path);
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

        let blob_content = git_blob_decoded_content(&ctx.cfg.repo_root, blob_sha).ok_or_else(|| {
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
            ctx.cfg,
            path,
            blob_sha,
            &classification.language,
            &classification.extraction_fingerprint,
            &blob_content,
        );
        let historical_file_record = commit_scoped_historical_artefact_record(
            &ctx.cfg.repo.repo_id,
            commit_sha,
            &file_record,
        );
        sql_batch.push(build_insert_historical_artefact_sql(
            ctx.cfg,
            ctx.relational,
            &file_artefact.language,
            &file_artefact.extraction_fingerprint,
            &historical_file_record,
        ));
        sql_batch.push(build_insert_commit_artefact_sql(
            ctx.relational,
            &ctx.cfg.repo.repo_id,
            commit_sha,
            path,
            blob_sha,
            &historical_file_record,
        ));
        artefacts_appended += 1;

        if classification.analysis_mode == AnalysisMode::Text || blob_content.decode_degraded {
            continue;
        }
        let source_content = blob_content.text.as_deref().unwrap_or_default();
        artefacts_appended += append_overlapping_language_artefact_metadata_sql(
            ctx.cfg,
            ctx.relational,
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
        ctx.relational
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
        let historical_record =
            commit_scoped_historical_artefact_record(&cfg.repo.repo_id, rev.commit_sha, record);
        sql_batch.push(build_insert_historical_artefact_sql(
            cfg,
            relational,
            &file_artefact.language,
            &file_artefact.extraction_fingerprint,
            &historical_record,
        ));
        sql_batch.push(build_insert_commit_artefact_sql(
            relational,
            &cfg.repo.repo_id,
            rev.commit_sha,
            rev.path,
            rev.blob_sha,
            &historical_record,
        ));
        appended += 1;
    }
    Ok(appended)
}

fn commit_scoped_historical_artefact_record(
    repo_id: &str,
    commit_sha: &str,
    record: &PersistedArtefactRecord,
) -> PersistedArtefactRecord {
    let mut scoped = record.clone();
    scoped.artefact_id =
        commit_scoped_historical_artefact_id(repo_id, commit_sha, &record.artefact_id);
    scoped.parent_artefact_id = record
        .parent_artefact_id
        .as_deref()
        .map(|parent_id| commit_scoped_historical_artefact_id(repo_id, commit_sha, parent_id));
    scoped
}

fn commit_scoped_historical_artefact_id(
    repo_id: &str,
    commit_sha: &str,
    revision_artefact_id: &str,
) -> String {
    deterministic_uuid(&format!(
        "historical-commit-artefact|{repo_id}|{commit_sha}|{revision_artefact_id}"
    ))
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

fn after_side_changed_file_revision_for_delta(delta: &CommitFileDelta) -> Option<(&str, &str)> {
    delta
        .path_after
        .as_deref()
        .zip(delta.new_blob_sha.as_deref())
}
