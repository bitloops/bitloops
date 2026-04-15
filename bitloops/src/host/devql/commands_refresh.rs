use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedSyncTaskMetadata {
    pub task_id: String,
    pub merged: bool,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostCommitArtefactRefreshStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub files_failed: usize,
    pub queued_task: Option<QueuedSyncTaskMetadata>,
}

impl PostCommitArtefactRefreshStats {
    pub(crate) fn completed_with_failures(&self) -> bool {
        self.queued_task.is_none() && self.files_failed > 0
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn inline_from_summary(files_seen: usize, summary: &SyncSummary) -> Self {
        Self {
            files_seen,
            files_indexed: summary.paths_added + summary.paths_changed,
            files_deleted: summary.paths_removed,
            files_failed: summary.parse_errors,
            queued_task: None,
        }
    }

    fn queued(files_seen: usize, queued: crate::daemon::DevqlTaskEnqueueResult) -> Self {
        Self {
            files_seen,
            files_indexed: 0,
            files_deleted: 0,
            files_failed: 0,
            queued_task: Some(QueuedSyncTaskMetadata {
                task_id: queued.task.task_id,
                merged: queued.merged,
                queue_position: queued.task.queue_position,
                tasks_ahead: queued.task.tasks_ahead,
            }),
        }
    }
}

pub async fn run_post_commit_artefact_refresh(
    cfg: &DevqlConfig,
    commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(
        cfg,
        changed_files,
        "post-commit",
        Some(crate::daemon::PostCommitSnapshotSpec {
            commit_sha: commit_sha.trim().to_string(),
            changed_paths: changed_files.to_vec(),
        }),
    )
    .await
}

pub async fn run_post_commit_checkpoint_projection_refresh(
    cfg: &DevqlConfig,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    let checkpoint_id = checkpoint_id.trim();
    if commit_sha.is_empty() || checkpoint_id.is_empty() {
        return Ok(());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-commit checkpoint projection refresh")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "git post-commit checkpoint projection refresh",
    )
    .await?;

    refresh_checkpoint_projection_for_commit(cfg, &relational, commit_sha, checkpoint_id).await
}

pub async fn run_post_merge_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-merge", None).await
}

async fn sync_changed_paths(
    cfg: &DevqlConfig,
    changed_files: &[String],
    source_hook: &str,
    post_commit_snapshot: Option<crate::daemon::PostCommitSnapshotSpec>,
) -> Result<PostCommitArtefactRefreshStats> {
    let mut paths = changed_files
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let paths = filter_refresh_paths_for_sync(cfg, &paths, source_hook)?;
    let post_commit_snapshot = post_commit_snapshot.map(|mut snapshot| {
        snapshot.changed_paths = paths.clone();
        snapshot
    });

    let stats = PostCommitArtefactRefreshStats {
        files_seen: paths.len(),
        ..PostCommitArtefactRefreshStats::default()
    };
    if paths.is_empty() {
        if let Some(snapshot) = post_commit_snapshot.as_ref() {
            snapshot_committed_current_rows_for_commit_for_config(cfg, snapshot)
                .await
                .with_context(|| {
                    format!("snapshotting committed current rows for {source_hook} refresh")
                })?;
        }
        return Ok(stats);
    }

    #[cfg(test)]
    {
        let summary = crate::host::devql::run_sync_with_summary(cfg, SyncMode::Paths(paths))
            .await
            .with_context(|| {
                format!("running DevQL sync inline for {source_hook} refresh in tests")
            })?;
        if let Some(snapshot) = post_commit_snapshot.as_ref() {
            snapshot_committed_current_rows_for_commit_for_config(cfg, snapshot)
                .await
                .with_context(|| {
                    format!("snapshotting committed current rows after {source_hook} sync in tests")
                })?;
        }
        Ok(PostCommitArtefactRefreshStats::inline_from_summary(
            stats.files_seen,
            &summary,
        ))
    }

    #[cfg(not(test))]
    {
        let source = match source_hook {
            "post-commit" => crate::daemon::DevqlTaskSource::PostCommit,
            "post-merge" => crate::daemon::DevqlTaskSource::PostMerge,
            _ => crate::daemon::DevqlTaskSource::ManualCli,
        };
        let queued = crate::daemon::enqueue_sync_for_config_with_snapshot(
            cfg,
            source,
            SyncMode::Paths(paths),
            post_commit_snapshot,
        )
        .with_context(|| format!("queueing DevQL sync for {source_hook} refresh"))?;
        Ok(PostCommitArtefactRefreshStats::queued(
            stats.files_seen,
            queued,
        ))
    }
}

pub(crate) async fn snapshot_committed_current_rows_for_commit_for_config(
    cfg: &DevqlConfig,
    snapshot: &crate::daemon::PostCommitSnapshotSpec,
) -> Result<()> {
    let commit_sha = snapshot.commit_sha.trim();
    if commit_sha.is_empty() {
        return Ok(());
    }
    let current_head = crate::host::checkpoints::strategy::manual_commit::head_hash(&cfg.repo_root)
        .context("resolving HEAD before post-commit semantic snapshot")?;
    if current_head.trim() != commit_sha {
        bail!(
            "skipping post-commit semantic snapshot for commit {} because repository HEAD is {}",
            commit_sha,
            current_head.trim()
        );
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-commit semantic snapshot")?;
    let relational =
        RelationalStorage::connect(cfg, &backends.relational, "post-commit semantic snapshot")
            .await?;
    snapshot_committed_current_rows_for_commit(
        &relational,
        &cfg.repo.repo_id,
        commit_sha,
        &snapshot.changed_paths,
    )
    .await
}

async fn snapshot_committed_current_rows_for_commit(
    relational: &RelationalStorage,
    repo_id: &str,
    commit_sha: &str,
    changed_paths: &[String],
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    if repo_id.trim().is_empty() || commit_sha.is_empty() {
        return Ok(());
    }

    let repo_id = esc_pg(repo_id);
    let commit_sha = esc_pg(commit_sha);
    let scoped_paths = changed_paths
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let current_scope_predicate = sql_optional_path_scope_predicate_pg("c.path", &scoped_paths);
    let clone_scope_predicate = sql_clone_scope_predicate_pg("src.path", "tgt.path", &scoped_paths);
    relational
        .exec_batch_transactional(&[
            format!(
                "INSERT INTO file_state (repo_id, commit_sha, path, blob_sha) \
                 SELECT c.repo_id, '{commit_sha}', c.path, c.effective_content_id \
                 FROM current_file_state c \
                 WHERE c.repo_id = '{repo_id}' \
                   AND c.effective_source = 'head' \
                   AND c.effective_content_id IS NOT NULL \
                 ON CONFLICT (repo_id, commit_sha, path) DO UPDATE SET blob_sha = EXCLUDED.blob_sha"
            ),
            format!(
                "INSERT INTO artefacts (
                    artefact_id, symbol_id, repo_id, language, canonical_kind, language_kind,
                    symbol_fqn, signature, modifiers, docstring, content_hash
                 )
                 SELECT
                    a.artefact_id, a.symbol_id, a.repo_id, a.language, a.canonical_kind,
                    a.language_kind, a.symbol_fqn, a.signature, a.modifiers, a.docstring,
                    a.content_id
                 FROM artefacts_current a
                 JOIN current_file_state c
                   ON c.repo_id = a.repo_id
                  AND c.path = a.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = a.content_id
                 WHERE a.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    symbol_id = EXCLUDED.symbol_id,
                    repo_id = EXCLUDED.repo_id,
                    language = EXCLUDED.language,
                    canonical_kind = EXCLUDED.canonical_kind,
                    language_kind = EXCLUDED.language_kind,
                    symbol_fqn = EXCLUDED.symbol_fqn,
                    signature = EXCLUDED.signature,
                    modifiers = EXCLUDED.modifiers,
                    docstring = EXCLUDED.docstring,
                    content_hash = EXCLUDED.content_hash",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO artefact_snapshots (
                    repo_id, blob_sha, path, artefact_id, parent_artefact_id,
                    start_line, end_line, start_byte, end_byte
                 )
                 SELECT
                    a.repo_id, a.content_id, a.path, a.artefact_id, a.parent_artefact_id,
                    a.start_line, a.end_line, a.start_byte, a.end_byte
                 FROM artefacts_current a
                 JOIN current_file_state c
                   ON c.repo_id = a.repo_id
                  AND c.path = a.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = a.content_id
                 WHERE a.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (repo_id, blob_sha, artefact_id) DO UPDATE SET
                    path = EXCLUDED.path,
                    parent_artefact_id = EXCLUDED.parent_artefact_id,
                    start_line = EXCLUDED.start_line,
                    end_line = EXCLUDED.end_line,
                    start_byte = EXCLUDED.start_byte,
                    end_byte = EXCLUDED.end_byte",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_semantics (
                    artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                    docstring_summary, llm_summary, template_summary, summary, confidence,
                    source_model
                 )
                 SELECT
                    s.artefact_id, s.repo_id, s.content_id, s.semantic_features_input_hash,
                    s.docstring_summary, s.llm_summary, s.template_summary, s.summary,
                    s.confidence, s.source_model
                 FROM symbol_semantics_current s
                 JOIN current_file_state c
                   ON c.repo_id = s.repo_id
                  AND c.path = s.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = s.content_id
                 WHERE s.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    semantic_features_input_hash = EXCLUDED.semantic_features_input_hash,
                    docstring_summary = EXCLUDED.docstring_summary,
                    llm_summary = EXCLUDED.llm_summary,
                    template_summary = EXCLUDED.template_summary,
                    summary = EXCLUDED.summary,
                    confidence = EXCLUDED.confidence,
                    source_model = EXCLUDED.source_model",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_features (
                    artefact_id, repo_id, blob_sha, semantic_features_input_hash,
                    normalized_name, normalized_signature, modifiers, identifier_tokens,
                    normalized_body_tokens, parent_kind, context_tokens
                 )
                 SELECT
                    f.artefact_id, f.repo_id, f.content_id, f.semantic_features_input_hash,
                    f.normalized_name, f.normalized_signature, f.modifiers,
                    f.identifier_tokens, f.normalized_body_tokens, f.parent_kind,
                    f.context_tokens
                 FROM symbol_features_current f
                 JOIN current_file_state c
                   ON c.repo_id = f.repo_id
                  AND c.path = f.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = f.content_id
                 WHERE f.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    semantic_features_input_hash = EXCLUDED.semantic_features_input_hash,
                    normalized_name = EXCLUDED.normalized_name,
                    normalized_signature = EXCLUDED.normalized_signature,
                    modifiers = EXCLUDED.modifiers,
                    identifier_tokens = EXCLUDED.identifier_tokens,
                    normalized_body_tokens = EXCLUDED.normalized_body_tokens,
                    parent_kind = EXCLUDED.parent_kind,
                    context_tokens = EXCLUDED.context_tokens",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_embeddings (
                    artefact_id, repo_id, blob_sha, representation_kind, setup_fingerprint,
                    provider, model, dimension, embedding_input_hash, embedding
                 )
                 SELECT
                    e.artefact_id, e.repo_id, e.content_id, e.representation_kind,
                    e.setup_fingerprint, e.provider, e.model, e.dimension,
                    e.embedding_input_hash, e.embedding
                 FROM symbol_embeddings_current e
                 JOIN current_file_state c
                   ON c.repo_id = e.repo_id
                  AND c.path = e.path
                  AND c.effective_source = 'head'
                  AND c.effective_content_id = e.content_id
                 WHERE e.repo_id = '{repo_id}'
                   {current_scope_predicate}
                 ON CONFLICT (artefact_id, representation_kind, setup_fingerprint) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    blob_sha = EXCLUDED.blob_sha,
                    provider = EXCLUDED.provider,
                    model = EXCLUDED.model,
                    dimension = EXCLUDED.dimension,
                    embedding_input_hash = EXCLUDED.embedding_input_hash,
                    embedding = EXCLUDED.embedding",
                current_scope_predicate = current_scope_predicate,
            ),
            format!(
                "INSERT INTO symbol_clone_edges (
                    repo_id, source_symbol_id, source_artefact_id, target_symbol_id,
                    target_artefact_id, relation_kind, score, semantic_score,
                    lexical_score, structural_score, clone_input_hash, explanation_json
                 )
                 SELECT
                    ce.repo_id, ce.source_symbol_id, ce.source_artefact_id,
                    ce.target_symbol_id, ce.target_artefact_id, ce.relation_kind, ce.score,
                    ce.semantic_score, ce.lexical_score, ce.structural_score,
                    ce.clone_input_hash, ce.explanation_json
                 FROM symbol_clone_edges_current ce
                 JOIN artefacts_current src
                   ON src.repo_id = ce.repo_id
                  AND src.artefact_id = ce.source_artefact_id
                 JOIN current_file_state src_state
                   ON src_state.repo_id = src.repo_id
                  AND src_state.path = src.path
                  AND src_state.effective_source = 'head'
                  AND src_state.effective_content_id = src.content_id
                 JOIN artefacts_current tgt
                   ON tgt.repo_id = ce.repo_id
                  AND tgt.artefact_id = ce.target_artefact_id
                 JOIN current_file_state tgt_state
                   ON tgt_state.repo_id = tgt.repo_id
                  AND tgt_state.path = tgt.path
                  AND tgt_state.effective_source = 'head'
                  AND tgt_state.effective_content_id = tgt.content_id
                 WHERE ce.repo_id = '{repo_id}'
                   {clone_scope_predicate}
                 ON CONFLICT (repo_id, source_artefact_id, target_artefact_id) DO UPDATE SET
                    source_symbol_id = EXCLUDED.source_symbol_id,
                    source_artefact_id = EXCLUDED.source_artefact_id,
                    target_symbol_id = EXCLUDED.target_symbol_id,
                    target_artefact_id = EXCLUDED.target_artefact_id,
                    relation_kind = EXCLUDED.relation_kind,
                    score = EXCLUDED.score,
                    semantic_score = EXCLUDED.semantic_score,
                    lexical_score = EXCLUDED.lexical_score,
                    structural_score = EXCLUDED.structural_score,
                    clone_input_hash = EXCLUDED.clone_input_hash,
                    explanation_json = EXCLUDED.explanation_json",
                clone_scope_predicate = clone_scope_predicate,
            ),
        ])
        .await
}

fn sql_optional_path_scope_predicate_pg(column: &str, paths: &[String]) -> String {
    if paths.is_empty() {
        String::new()
    } else {
        format!("AND {column} IN ({})", sql_string_list_pg(paths))
    }
}

fn sql_clone_scope_predicate_pg(
    source_column: &str,
    target_column: &str,
    paths: &[String],
) -> String {
    if paths.is_empty() {
        String::new()
    } else {
        format!(
            "AND ({source_column} IN ({paths_sql}) OR {target_column} IN ({paths_sql}))",
            paths_sql = sql_string_list_pg(paths),
        )
    }
}

fn filter_refresh_paths_for_sync(
    cfg: &DevqlConfig,
    paths: &[String],
    source_hook: &str,
) -> Result<Vec<String>> {
    let exclusion_matcher = load_repo_exclusion_matcher(&cfg.repo_root).with_context(|| {
        format!("loading repo policy exclusions for {source_hook} path refresh")
    })?;
    let (parser_version, extractor_version) =
        resolve_pack_versions_for_refresh().with_context(|| {
            format!("resolving language pack versions for {source_hook} path refresh")
        })?;
    let classifier = ProjectAwareClassifier::discover_for_worktree(
        &cfg.repo_root,
        paths.iter().map(String::as_str),
        &parser_version,
        &extractor_version,
    )
    .with_context(|| format!("building classifier for {source_hook} path refresh"))?;
    let mut filtered = Vec::new();
    for path in paths {
        let classification = classifier
            .classify_repo_relative_path(path, exclusion_matcher.excludes_repo_relative_path(path))
            .with_context(|| format!("classifying {source_hook} refresh path `{path}`"))?;
        if classification.analysis_mode != AnalysisMode::Excluded {
            filtered.push(path.clone());
        }
    }
    Ok(filtered)
}

fn resolve_pack_versions_for_refresh() -> Result<(String, String)> {
    let host = core_extension_host()?;
    let mut packs = host
        .language_packs()
        .registered_pack_ids()
        .into_iter()
        .filter_map(|pack_id| host.language_packs().resolve_pack(pack_id))
        .map(|descriptor| format!("{}@{}", descriptor.id, descriptor.version))
        .collect::<Vec<_>>();
    packs.sort();
    let joined = packs.join("+");
    Ok((
        format!("devql-sync-parser@{joined}"),
        format!("devql-sync-extractor@{joined}"),
    ))
}

async fn refresh_checkpoint_projection_for_commit(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    ensure_repository_row(cfg, relational).await?;

    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
        &cfg.repo_root,
        checkpoint_id,
    )?
    .ok_or_else(|| {
        anyhow::anyhow!("checkpoint not found for projection refresh: {checkpoint_id}")
    })?;
    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, commit_sha);

    let _projected_rows = upsert_checkpoint_file_snapshot_rows(
        cfg,
        relational,
        &checkpoint,
        commit_sha,
        commit_info.as_ref(),
    )
    .await?;

    Ok(())
}

pub async fn run_post_checkout_branch_seed(
    cfg: &DevqlConfig,
    _previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout || new_head.trim().is_empty() || is_zero_git_oid(new_head) {
        return Ok(());
    }

    #[cfg(test)]
    {
        crate::host::devql::run_sync_with_summary(cfg, SyncMode::Full)
            .await
            .context("running full DevQL sync inline for post-checkout branch seed in tests")?;
        Ok(())
    }

    #[cfg(not(test))]
    {
        crate::daemon::enqueue_sync_for_config(
            cfg,
            crate::daemon::DevqlTaskSource::PostCheckout,
            SyncMode::Full,
        )
        .context("queueing full DevQL sync for post-checkout branch seed")?;
        Ok(())
    }
}

fn is_zero_git_oid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{
        DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec,
        DevqlTaskStatus, SyncTaskMode, SyncTaskSpec,
    };
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn sample_queued_result() -> crate::daemon::DevqlTaskEnqueueResult {
        crate::daemon::DevqlTaskEnqueueResult {
            task: DevqlTaskRecord {
                task_id: "sync-task-123".to_string(),
                repo_id: "repo-1".to_string(),
                repo_name: "demo".to_string(),
                repo_provider: "local".to_string(),
                repo_organisation: "local".to_string(),
                repo_identity: "local/demo".to_string(),
                daemon_config_root: PathBuf::from("/tmp/repo"),
                repo_root: PathBuf::from("/tmp/repo"),
                kind: DevqlTaskKind::Sync,
                source: DevqlTaskSource::PostCommit,
                spec: DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Paths {
                        paths: vec!["src/lib.rs".to_string()],
                    },
                    post_commit_snapshot: None,
                }),
                status: DevqlTaskStatus::Queued,
                submitted_at_unix: 1,
                started_at_unix: None,
                updated_at_unix: 1,
                completed_at_unix: None,
                queue_position: Some(3),
                tasks_ahead: Some(2),
                progress: DevqlTaskProgress::Sync(SyncProgressUpdate::default()),
                error: None,
                result: None,
            },
            merged: true,
        }
    }

    #[test]
    fn queued_refresh_stats_include_task_metadata() {
        let stats = PostCommitArtefactRefreshStats::queued(2, sample_queued_result());

        assert_eq!(stats.files_seen, 2);
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_deleted, 0);
        assert_eq!(stats.files_failed, 0);
        assert_eq!(
            stats.queued_task,
            Some(QueuedSyncTaskMetadata {
                task_id: "sync-task-123".to_string(),
                merged: true,
                queue_position: Some(3),
                tasks_ahead: Some(2),
            })
        );
        assert!(!stats.completed_with_failures());
    }

    #[test]
    fn inline_refresh_stats_report_completed_failures() {
        let stats = PostCommitArtefactRefreshStats {
            files_seen: 2,
            files_indexed: 1,
            files_deleted: 0,
            files_failed: 1,
            queued_task: None,
        };

        assert!(stats.completed_with_failures());
    }

    #[tokio::test]
    async fn snapshot_committed_current_rows_for_commit_promotes_only_head_rows() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("relational.sqlite");
        crate::host::devql::init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite schema");
        let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());
        let repo_id = "repo-refresh-test";

        relational
            .exec_batch_transactional(&[
                format!(
                    "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
                     VALUES ('{repo_id}', 'local', 'local', 'demo', 'main')"
                ),
                format!(
                    "INSERT INTO current_file_state (
                        repo_id, path, analysis_mode, file_role, text_index_mode, language,
                        resolved_language, secondary_context_ids_json, frameworks_json,
                        classification_reason, extraction_fingerprint, head_content_id,
                        index_content_id, worktree_content_id, effective_content_id,
                        effective_source, parser_version, extractor_version, exists_in_head,
                        exists_in_index, exists_in_worktree, last_synced_at
                     ) VALUES
                        ('{repo_id}', 'src/head_a.ts', 'code', 'source_code', 'none', 'typescript',
                         'typescript', '[]', '[]', 'test', 'fp-a', 'blob-a', 'blob-a', 'blob-a',
                         'blob-a', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/head_b.ts', 'code', 'source_code', 'none', 'typescript',
                         'typescript', '[]', '[]', 'test', 'fp-b', 'blob-b', 'blob-b', 'blob-b',
                         'blob-b', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/draft.ts', 'code', 'source_code', 'none', 'typescript',
                         'typescript', '[]', '[]', 'test', 'fp-draft', 'blob-draft-head', 'blob-draft-index', 'blob-draft-worktree',
                         'blob-draft-worktree', 'worktree', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z')"
                ),
                format!(
                    "INSERT INTO artefacts_current (
                        repo_id, path, content_id, symbol_id, artefact_id, language,
                        extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                        parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte,
                        end_byte, signature, modifiers, docstring, updated_at
                     ) VALUES
                        ('{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'artefact::a', 'typescript',
                         'fp-a', 'function', 'function_declaration', 'src/head_a.ts::renderA',
                         NULL, NULL, 1, 4, 0, 40, 'function renderA()', '[]', NULL, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'artefact::b', 'typescript',
                         'fp-b', 'function', 'function_declaration', 'src/head_b.ts::renderB',
                         NULL, NULL, 1, 4, 0, 40, 'function renderB()', '[]', NULL, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'artefact::draft', 'typescript',
                         'fp-draft', 'function', 'function_declaration', 'src/draft.ts::renderDraft',
                         NULL, NULL, 1, 4, 0, 40, 'function renderDraft()', '[]', NULL, '2026-04-15T10:00:00Z')"
                ),
                format!(
                    "INSERT INTO symbol_semantics_current (
                        artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                        template_summary, summary, confidence
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                         'Head A summary', 'Head A summary', 0.9),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                         'Head B summary', 'Head B summary', 0.9),
                        ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'hash-draft',
                         'Draft summary', 'Draft summary', 0.9)"
                ),
                format!(
                    "INSERT INTO symbol_features_current (
                        artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                        normalized_name, normalized_signature, modifiers, identifier_tokens,
                        normalized_body_tokens, parent_kind, context_tokens
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                         'render_a', 'function renderA()', '[]', '[\"render\",\"a\"]', '[\"render\",\"a\"]', 'module', '[\"head\",\"a\"]'),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                         'render_b', 'function renderB()', '[]', '[\"render\",\"b\"]', '[\"render\",\"b\"]', 'module', '[\"head\",\"b\"]'),
                        ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'hash-draft',
                         'render_draft', 'function renderDraft()', '[]', '[\"render\",\"draft\"]', '[\"render\",\"draft\"]', 'module', '[\"draft\"]')"
                ),
                "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
                     VALUES ('setup-code', 'local', 'test-model', 3)"
                    .to_string(),
                format!(
                    "INSERT INTO symbol_embeddings_current (
                        artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                        setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'code',
                         'setup-code', 'local', 'test-model', 3, 'embed-a', '[0.1,0.2,0.3]'),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'code',
                         'setup-code', 'local', 'test-model', 3, 'embed-b', '[0.2,0.1,0.3]'),
                        ('artefact::draft', '{repo_id}', 'src/draft.ts', 'blob-draft-worktree', 'sym::draft', 'code',
                         'setup-code', 'local', 'test-model', 3, 'embed-draft', '[0.3,0.2,0.1]')"
                ),
                format!(
                    "INSERT INTO symbol_clone_edges_current (
                        repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                        relation_kind, score, semantic_score, lexical_score, structural_score,
                        clone_input_hash, explanation_json
                     ) VALUES
                        ('{repo_id}', 'sym::a', 'artefact::a', 'sym::b', 'artefact::b',
                         'similar_implementation', 0.91, 0.9, 0.8, 0.7, 'clone-head', '{{}}'),
                        ('{repo_id}', 'sym::a', 'artefact::a', 'sym::draft', 'artefact::draft',
                         'similar_implementation', 0.51, 0.5, 0.4, 0.3, 'clone-draft', '{{}}')"
                ),
            ])
            .await
            .expect("seed current projection rows");

        snapshot_committed_current_rows_for_commit(
            &relational,
            repo_id,
            "commit-head",
            &[
                "src/head_a.ts".to_string(),
                "src/head_b.ts".to_string(),
                "src/draft.ts".to_string(),
            ],
        )
        .await
        .expect("snapshot committed current rows");

        let db = Connection::open(&sqlite_path).expect("open sqlite db");
        let file_state_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
                rusqlite::params![repo_id, "commit-head"],
                |row| row.get(0),
            )
            .expect("count file_state rows");
        let artefact_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical artefacts");
        let snapshot_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM artefact_snapshots WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count artefact snapshots");
        let semantic_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical semantic rows");
        let feature_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_features WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical feature rows");
        let embedding_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical embedding rows");
        let clone_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical clone rows");
        let draft_file_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2 AND path = ?3",
                rusqlite::params![repo_id, "commit-head", "src/draft.ts"],
                |row| row.get(0),
            )
            .expect("count draft file_state rows");
        let draft_embedding_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1 AND blob_sha = ?2",
                rusqlite::params![repo_id, "blob-draft-worktree"],
                |row| row.get(0),
            )
            .expect("count draft embedding rows");

        assert_eq!(file_state_rows, 2);
        assert_eq!(artefact_rows, 2);
        assert_eq!(snapshot_rows, 2);
        assert_eq!(semantic_rows, 2);
        assert_eq!(feature_rows, 2);
        assert_eq!(embedding_rows, 2);
        assert_eq!(clone_rows, 1);
        assert_eq!(draft_file_rows, 0);
        assert_eq!(draft_embedding_rows, 0);
    }

    #[tokio::test]
    async fn snapshot_committed_current_rows_for_commit_limits_semantic_history_to_changed_paths() {
        let temp = tempdir().expect("temp dir");
        let sqlite_path = temp.path().join("relational.sqlite");
        crate::host::devql::init_sqlite_schema(&sqlite_path)
            .await
            .expect("initialise sqlite schema");
        let relational = crate::host::devql::RelationalStorage::local_only(sqlite_path.clone());
        let repo_id = "repo-refresh-scope-test";

        relational
            .exec_batch_transactional(&[
                format!(
                    "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) \
                     VALUES ('{repo_id}', 'local', 'local', 'demo', 'main')"
                ),
                format!(
                    "INSERT INTO current_file_state (
                        repo_id, path, analysis_mode, file_role, text_index_mode, language,
                        resolved_language, secondary_context_ids_json, frameworks_json,
                        classification_reason, extraction_fingerprint, head_content_id,
                        index_content_id, worktree_content_id, effective_content_id,
                        effective_source, parser_version, extractor_version, exists_in_head,
                        exists_in_index, exists_in_worktree, last_synced_at
                     ) VALUES
                        ('{repo_id}', 'src/head_a.ts', 'code', 'source_code', 'none', 'typescript',
                         'typescript', '[]', '[]', 'test', 'fp-a', 'blob-a', 'blob-a', 'blob-a',
                         'blob-a', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/head_b.ts', 'code', 'source_code', 'none', 'typescript',
                         'typescript', '[]', '[]', 'test', 'fp-b', 'blob-b', 'blob-b', 'blob-b',
                         'blob-b', 'head', 'parser', 'extractor', 1, 1, 1, '2026-04-15T10:00:00Z')"
                ),
                format!(
                    "INSERT INTO artefacts_current (
                        repo_id, path, content_id, symbol_id, artefact_id, language,
                        extraction_fingerprint, canonical_kind, language_kind, symbol_fqn,
                        parent_symbol_id, parent_artefact_id, start_line, end_line, start_byte,
                        end_byte, signature, modifiers, docstring, updated_at
                     ) VALUES
                        ('{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'artefact::a', 'typescript',
                         'fp-a', 'function', 'function_declaration', 'src/head_a.ts::renderA',
                         NULL, NULL, 1, 4, 0, 40, 'function renderA()', '[]', NULL, '2026-04-15T10:00:00Z'),
                        ('{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'artefact::b', 'typescript',
                         'fp-b', 'function', 'function_declaration', 'src/head_b.ts::renderB',
                         NULL, NULL, 1, 4, 0, 40, 'function renderB()', '[]', NULL, '2026-04-15T10:00:00Z')"
                ),
                format!(
                    "INSERT INTO symbol_semantics_current (
                        artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                        template_summary, summary, confidence
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                         'Head A summary', 'Head A summary', 0.9),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                         'Head B summary', 'Head B summary', 0.9)"
                ),
                format!(
                    "INSERT INTO symbol_features_current (
                        artefact_id, repo_id, path, content_id, symbol_id, semantic_features_input_hash,
                        normalized_name, normalized_signature, modifiers, identifier_tokens,
                        normalized_body_tokens, parent_kind, context_tokens
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'hash-a',
                         'render_a', 'function renderA()', '[]', '[\"render\",\"a\"]', '[\"render\",\"a\"]', 'module', '[\"head\",\"a\"]'),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'hash-b',
                         'render_b', 'function renderB()', '[]', '[\"render\",\"b\"]', '[\"render\",\"b\"]', 'module', '[\"head\",\"b\"]')"
                ),
                "INSERT INTO semantic_embedding_setups (setup_fingerprint, provider, model, dimension)
                     VALUES ('setup-code', 'local', 'test-model', 3)"
                    .to_string(),
                format!(
                    "INSERT INTO symbol_embeddings_current (
                        artefact_id, repo_id, path, content_id, symbol_id, representation_kind,
                        setup_fingerprint, provider, model, dimension, embedding_input_hash, embedding
                     ) VALUES
                        ('artefact::a', '{repo_id}', 'src/head_a.ts', 'blob-a', 'sym::a', 'code',
                         'setup-code', 'local', 'test-model', 3, 'embed-a', '[0.1,0.2,0.3]'),
                        ('artefact::b', '{repo_id}', 'src/head_b.ts', 'blob-b', 'sym::b', 'code',
                         'setup-code', 'local', 'test-model', 3, 'embed-b', '[0.2,0.1,0.3]')"
                ),
                format!(
                    "INSERT INTO symbol_clone_edges_current (
                        repo_id, source_symbol_id, source_artefact_id, target_symbol_id, target_artefact_id,
                        relation_kind, score, semantic_score, lexical_score, structural_score,
                        clone_input_hash, explanation_json
                     ) VALUES
                        ('{repo_id}', 'sym::a', 'artefact::a', 'sym::b', 'artefact::b',
                         'similar_implementation', 0.91, 0.9, 0.8, 0.7, 'clone-head', '{{}}')"
                ),
            ])
            .await
            .expect("seed current projection rows");

        snapshot_committed_current_rows_for_commit(
            &relational,
            repo_id,
            "commit-head",
            &["src/head_a.ts".to_string()],
        )
        .await
        .expect("snapshot scoped current rows");

        let db = Connection::open(&sqlite_path).expect("open sqlite db");
        let file_state_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
                rusqlite::params![repo_id, "commit-head"],
                |row| row.get(0),
            )
            .expect("count file_state rows");
        let artefact_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM artefacts WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical artefacts");
        let semantic_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_semantics WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical semantic rows");
        let embedding_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_embeddings WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical embedding rows");
        let clone_rows: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM symbol_clone_edges WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get(0),
            )
            .expect("count historical clone rows");

        assert_eq!(file_state_rows, 2);
        assert_eq!(artefact_rows, 1);
        assert_eq!(semantic_rows, 1);
        assert_eq!(embedding_rows, 1);
        assert_eq!(clone_rows, 1);
    }
}
