use super::*;

use chrono::{TimeZone, Utc};
use std::collections::BTreeMap;

use crate::host::checkpoints::strategy::manual_commit::resolve_default_branch_name;

// Checkpoint and commit row persistence: mapping, event insertion, upserts.

pub(super) async fn ensure_repository_row(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<()> {
    let role = RelationalStorageRole::SharedRelational;
    let metadata_json = build_repository_metadata_json(cfg)
        .context("building repository metadata profile for DevQL persistence")?;
    let sql_with_metadata = format!(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch, metadata_json) VALUES ('{}', '{}', '{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id) DO UPDATE SET provider = EXCLUDED.provider, organization = EXCLUDED.organization, name = EXCLUDED.name, default_branch = EXCLUDED.default_branch, metadata_json = EXCLUDED.metadata_json",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&cfg.repo.provider),
        esc_pg(&cfg.repo.organization),
        esc_pg(&cfg.repo.name),
        esc_pg(&default_branch_name(&cfg.repo_root)),
        esc_pg(&metadata_json),
    );
    if let Err(err) = relational.exec_for_role(role, &sql_with_metadata).await {
        let message = format!("{err:#}");
        let missing_metadata_column = message.contains("no column named metadata_json")
            || message.contains("column \"metadata_json\" does not exist");
        if !missing_metadata_column {
            return Err(err);
        }

        let legacy_sql = format!(
            "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) VALUES ('{}', '{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id) DO UPDATE SET provider = EXCLUDED.provider, organization = EXCLUDED.organization, name = EXCLUDED.name, default_branch = EXCLUDED.default_branch",
            esc_pg(&cfg.repo.repo_id),
            esc_pg(&cfg.repo.provider),
            esc_pg(&cfg.repo.organization),
            esc_pg(&cfg.repo.name),
            esc_pg(&default_branch_name(&cfg.repo_root)),
        );
        return relational.exec_for_role(role, &legacy_sql).await;
    }
    Ok(())
}

fn build_repository_metadata_json(cfg: &DevqlConfig) -> Result<String> {
    let exclusion_matcher = load_repo_exclusion_matcher(&cfg.repo_root)
        .context("loading repo policy exclusions for repository metadata")?;
    let (parser_version, extractor_version) = resolve_pack_versions_for_repository_metadata()
        .context("resolving language pack versions for repository metadata")?;
    let tracked = run_git(&cfg.repo_root, &["ls-files", "-z"]).unwrap_or_default();
    let tracked_paths = tracked
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(normalize_repo_path)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let classifier = ProjectAwareClassifier::discover_for_worktree(
        &cfg.repo_root,
        tracked_paths.clone(),
        &parser_version,
        &extractor_version,
    )
    .context("building project-aware classifier for repository metadata")?;
    let mut language_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut role_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut text_index_mode_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut text_file_count = 0usize;
    let mut track_only_file_count = 0usize;

    for path in tracked_paths {
        let classification = classifier
            .classify_repo_relative_path(
                &path,
                exclusion_matcher.excludes_repo_relative_path(&path),
            )
            .with_context(|| format!("classifying repository metadata path `{path}`"))?;
        match classification.analysis_mode {
            AnalysisMode::Code => {
                *role_counts
                    .entry(classification.file_role.as_str().to_string())
                    .or_insert(0) += 1;
                *language_counts.entry(classification.language).or_insert(0) += 1;
            }
            AnalysisMode::Text => {
                *role_counts
                    .entry(classification.file_role.as_str().to_string())
                    .or_insert(0) += 1;
                *text_index_mode_counts
                    .entry(classification.text_index_mode.as_str().to_string())
                    .or_insert(0) += 1;
                text_file_count += 1;
            }
            AnalysisMode::TrackOnly => {
                *role_counts
                    .entry(classification.file_role.as_str().to_string())
                    .or_insert(0) += 1;
                track_only_file_count += 1;
            }
            AnalysisMode::Excluded => {}
        }
    }

    let contexts = classifier
        .contexts()
        .into_iter()
        .map(|context| {
            serde_json::json!({
                "context_id": context.context_id,
                "root": context.root,
                "kind": context.kind,
                "detection_source": context.detection_source,
                "config_files": context.config_files,
                "config_fingerprint": context.config_fingerprint,
                "base_languages": context.base_languages,
                "frameworks": context.frameworks,
                "runtime_profile": context.runtime_profile,
                "source_versions": context.source_versions,
            })
        })
        .collect::<Vec<_>>();

    let languages = language_counts.keys().cloned().collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "contexts": contexts,
        "language_profile": {
            "languages": languages,
            "file_count_by_language": language_counts,
            "file_count_by_role": role_counts,
            "text_file_count": text_file_count,
            "text_file_count_by_index_mode": text_index_mode_counts,
            "track_only_file_count": track_only_file_count,
        }
    }))
    .context("serialising repository metadata JSON")
}

fn resolve_pack_versions_for_repository_metadata() -> Result<(String, String)> {
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

pub(super) fn default_branch_name(repo_root: &Path) -> String {
    resolve_default_branch_name(repo_root)
}

pub(super) fn collect_checkpoint_commit_map(
    repo_root: &Path,
) -> Result<HashMap<String, CheckpointCommitInfo>> {
    collect_checkpoint_commit_map_from_db(repo_root)
}

pub(super) fn collect_checkpoint_commit_map_from_db(
    repo_root: &Path,
) -> Result<HashMap<String, CheckpointCommitInfo>> {
    let mappings = read_commit_checkpoint_mappings(repo_root)?;
    let mut out: HashMap<String, CheckpointCommitInfo> = HashMap::new();

    for (commit_sha, checkpoint_id) in mappings {
        let Some(info) = checkpoint_commit_info_from_sha(repo_root, &commit_sha) else {
            continue;
        };

        let should_replace = match out.get(&checkpoint_id) {
            None => true,
            Some(existing) => {
                info.commit_unix > existing.commit_unix
                    || (info.commit_unix == existing.commit_unix
                        && is_newer_commit_sha(repo_root, &existing.commit_sha, &info.commit_sha))
            }
        };
        if should_replace {
            out.insert(checkpoint_id, info);
        }
    }

    Ok(out)
}

pub(super) fn is_newer_commit_sha(
    repo_root: &Path,
    existing_sha: &str,
    candidate_sha: &str,
) -> bool {
    if existing_sha == candidate_sha {
        return false;
    }
    if commit_is_ancestor_of(repo_root, existing_sha, candidate_sha) {
        return true;
    }
    if commit_is_ancestor_of(repo_root, candidate_sha, existing_sha) {
        return false;
    }
    candidate_sha > existing_sha
}

pub(super) fn commit_is_ancestor_of(
    repo_root: &Path,
    ancestor_sha: &str,
    descendant_sha: &str,
) -> bool {
    run_git(
        repo_root,
        &["merge-base", "--is-ancestor", ancestor_sha, descendant_sha],
    )
    .is_ok()
}

pub(super) fn checkpoint_commit_info_from_sha(
    repo_root: &Path,
    commit_sha: &str,
) -> Option<CheckpointCommitInfo> {
    if commit_sha.trim().is_empty() {
        return None;
    }

    let raw = run_git(
        repo_root,
        &["show", "-s", "--format=%ct%x1f%an%x1f%ae%x1f%s", commit_sha],
    )
    .ok()?;

    let mut parts = raw.trim().splitn(4, '\u{1f}');
    let commit_unix = parts
        .next()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let author_name = parts.next().unwrap_or_default().trim().to_string();
    let author_email = parts.next().unwrap_or_default().trim().to_string();
    let subject = parts.next().unwrap_or_default().trim().to_string();

    Some(CheckpointCommitInfo {
        commit_sha: commit_sha.to_string(),
        commit_unix,
        author_name,
        author_email,
        subject,
    })
}

#[derive(Debug, Clone)]
pub(super) struct CheckpointEventsStore {
    inner: CheckpointEventsStoreInner,
}

#[derive(Debug, Clone)]
pub(super) enum CheckpointEventsStoreInner {
    ClickHouse {
        endpoint: String,
        user: Option<String>,
        password: Option<String>,
    },
    DuckDb {
        path: PathBuf,
    },
}

impl CheckpointEventsStore {
    fn from_config(cfg: &DevqlConfig, events_cfg: &EventsBackendConfig) -> Self {
        if events_cfg.has_clickhouse() {
            Self {
                inner: CheckpointEventsStoreInner::ClickHouse {
                    endpoint: cfg.clickhouse_endpoint(),
                    user: cfg.clickhouse_user.clone(),
                    password: cfg.clickhouse_password.clone(),
                },
            }
        } else {
            Self {
                inner: CheckpointEventsStoreInner::DuckDb {
                    path: events_cfg.duckdb_path_or_default(),
                },
            }
        }
    }

    async fn fetch_existing_event_ids(&self, repo_id: &str) -> Result<HashSet<String>> {
        match &self.inner {
            CheckpointEventsStoreInner::ClickHouse {
                endpoint,
                user,
                password,
            } => {
                let sql = format!(
                    "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}' FORMAT JSON",
                    esc_ch(repo_id)
                );
                let raw =
                    run_clickhouse_sql_http(endpoint, user.as_deref(), password.as_deref(), &sql)
                        .await?;
                let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
                    format!(
                        "parsing ClickHouse JSON response: {}",
                        truncate_for_error(&raw)
                    )
                })?;
                let mut out = HashSet::new();
                if let Some(rows) = parsed.get("data").and_then(Value::as_array) {
                    for row in rows {
                        if let Some(id) = row.get("event_id").and_then(Value::as_str) {
                            out.insert(id.to_string());
                        }
                    }
                }
                Ok(out)
            }
            CheckpointEventsStoreInner::DuckDb { path } => {
                let sql = format!(
                    "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}'",
                    esc_pg(repo_id)
                );
                let rows = duckdb_query_rows_path(path, &sql).await?;
                Ok(rows
                    .into_iter()
                    .filter_map(|row| {
                        row.get("event_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect())
            }
        }
    }

    async fn insert_checkpoint_event(
        &self,
        repo_id: &str,
        cp: &CommittedInfo,
        event_id: &str,
        commit_info: Option<&CheckpointCommitInfo>,
    ) -> Result<()> {
        let event_time = checkpoint_event_time_rfc3339(cp, commit_info);
        let commit_sha = commit_info
            .map(|info| info.commit_sha.as_str())
            .unwrap_or_default();
        let payload = json!({
            "checkpoints_count": cp.checkpoints_count,
            "session_count": cp.session_count,
            "token_usage": cp.token_usage,
        });
        let payload_json = serde_json::to_string(&payload)?;
        let files_touched_json = serde_json::to_string(&cp.files_touched)?;

        match &self.inner {
            CheckpointEventsStoreInner::ClickHouse {
                endpoint,
                user,
                password,
            } => {
                let files_touched = format_ch_array(&cp.files_touched);
                let sql = format!(
                    "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched, payload) \
VALUES ('{}', coalesce(parseDateTime64BestEffortOrNull('{}'), now64(3)), '{}', '{}', '{}', '{}', '{}', 'checkpoint_committed', '{}', '{}', {}, '{}')",
                    esc_ch(event_id),
                    esc_ch(&event_time),
                    esc_ch(repo_id),
                    esc_ch(&cp.checkpoint_id),
                    esc_ch(&cp.session_id),
                    esc_ch(commit_sha),
                    esc_ch(&cp.branch),
                    esc_ch(&cp.agent),
                    esc_ch(&cp.strategy),
                    files_touched,
                    esc_ch(&payload_json),
                );
                run_clickhouse_sql_http(endpoint, user.as_deref(), password.as_deref(), &sql)
                    .await
                    .map(|_| ())
            }
            CheckpointEventsStoreInner::DuckDb { path } => {
                let sql = format!(
                    "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched, payload) \
SELECT '{event_id}', '{event_time}', '{repo_id}', '{checkpoint_id}', '{session_id}', '{commit_sha}', '{branch}', 'checkpoint_committed', '{agent}', '{strategy}', '{files_touched}', '{payload}' \
WHERE NOT EXISTS (SELECT 1 FROM checkpoint_events WHERE event_id = '{event_id}')",
                    event_id = esc_pg(event_id),
                    event_time = esc_pg(&event_time),
                    repo_id = esc_pg(repo_id),
                    checkpoint_id = esc_pg(&cp.checkpoint_id),
                    session_id = esc_pg(&cp.session_id),
                    commit_sha = esc_pg(commit_sha),
                    branch = esc_pg(&cp.branch),
                    agent = esc_pg(&cp.agent),
                    strategy = esc_pg(&cp.strategy),
                    files_touched = esc_pg(&files_touched_json),
                    payload = esc_pg(&payload_json),
                );
                duckdb_exec_path(path, &sql).await
            }
        }
    }
}

pub(super) fn checkpoint_event_time_rfc3339(
    cp: &CommittedInfo,
    commit_info: Option<&CheckpointCommitInfo>,
) -> String {
    let created_at = cp.created_at.trim();
    if !created_at.is_empty() {
        return created_at.to_string();
    }

    if let Some(info) = commit_info
        && let Some(timestamp) = Utc.timestamp_opt(info.commit_unix, 0).single()
    {
        return timestamp.to_rfc3339();
    }

    Utc::now().to_rfc3339()
}

pub(super) async fn fetch_existing_checkpoint_event_ids(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
) -> Result<HashSet<String>> {
    CheckpointEventsStore::from_config(cfg, events_cfg)
        .fetch_existing_event_ids(&cfg.repo.repo_id)
        .await
}

pub(super) async fn insert_checkpoint_event(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    cp: &CommittedInfo,
    event_id: &str,
    commit_info: Option<&CheckpointCommitInfo>,
) -> Result<()> {
    CheckpointEventsStore::from_config(cfg, events_cfg)
        .insert_checkpoint_event(&cfg.repo.repo_id, cp, event_id, commit_info)
        .await
}

pub(super) async fn upsert_checkpoint_file_snapshot_rows(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    cp: &CommittedInfo,
    commit_sha: &str,
    commit_info: Option<&CheckpointCommitInfo>,
) -> Result<usize> {
    let commit_sha = commit_sha.trim();
    if commit_sha.is_empty() {
        return Ok(0);
    }

    let event_time = checkpoint_event_time_rfc3339(cp, commit_info);
    let context = crate::host::devql::checkpoint_provenance::CheckpointProvenanceContext {
        repo_id: &cfg.repo.repo_id,
        checkpoint_id: &cp.checkpoint_id,
        session_id: &cp.session_id,
        event_time: &event_time,
        agent: &cp.agent,
        branch: &cp.branch,
        strategy: &cp.strategy,
        commit_sha,
    };
    let file_rows =
        crate::host::devql::checkpoint_provenance::collect_checkpoint_file_provenance_rows(
            &cfg.repo_root,
            context,
        )?;
    let artefact_provenance =
        crate::host::devql::checkpoint_provenance::collect_checkpoint_artefact_provenance(
            &cfg.repo_root,
            context,
            &file_rows,
        )?;

    let shared_dialect = relational.dialect_for_role(RelationalStorageRole::SharedRelational);
    let mut sqlite_statements = Vec::with_capacity(
        3 + file_rows.len()
            + artefact_provenance.semantic_rows.len()
            + artefact_provenance.lineage_rows.len(),
    );
    sqlite_statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_lineage_rows_sql(
            &cfg.repo.repo_id,
            &cp.checkpoint_id,
        ),
    );
    sqlite_statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_rows_sql(
            &cfg.repo.repo_id,
            &cp.checkpoint_id,
        ),
    );
    sqlite_statements.push(
        crate::host::devql::checkpoint_provenance::delete_checkpoint_file_rows_sql(
            &cfg.repo.repo_id,
            &cp.checkpoint_id,
        ),
    );
    for row in &file_rows {
        sqlite_statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_file_row_sql(
                row,
                shared_dialect,
            ),
        );
    }
    for row in &artefact_provenance.semantic_rows {
        sqlite_statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                row,
                shared_dialect,
            ),
        );
    }
    for row in &artefact_provenance.lineage_rows {
        sqlite_statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_lineage_row_sql(
                row,
                shared_dialect,
            ),
        );
    }
    relational
        .exec_batch_transactional_for_role(
            RelationalStorageRole::SharedRelational,
            &sqlite_statements,
        )
        .await?;

    Ok(file_rows.len())
}
