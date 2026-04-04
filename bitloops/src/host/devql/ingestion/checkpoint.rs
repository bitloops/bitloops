use super::*;

use chrono::{TimeZone, Utc};

use crate::host::checkpoints::strategy::manual_commit::resolve_default_branch_name;

// Checkpoint and commit row persistence: mapping, event insertion, upserts.

pub(super) async fn ensure_repository_row(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO repositories (repo_id, provider, organization, name, default_branch) VALUES ('{}', '{}', '{}', '{}', '{}') \
ON CONFLICT (repo_id) DO UPDATE SET provider = EXCLUDED.provider, organization = EXCLUDED.organization, name = EXCLUDED.name, default_branch = EXCLUDED.default_branch",
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&cfg.repo.provider),
        esc_pg(&cfg.repo.organization),
        esc_pg(&cfg.repo.name),
        esc_pg(&default_branch_name(&cfg.repo_root))
    );
    relational.exec(&sql).await
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

pub(super) async fn upsert_commit_row(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    cp: &CommittedInfo,
    commit_info: &CheckpointCommitInfo,
) -> Result<()> {
    let committed_at_sql = match relational.dialect() {
        RelationalDialect::Postgres => format!("to_timestamp({})", commit_info.commit_unix),
        RelationalDialect::Sqlite => {
            format!("datetime({}, 'unixepoch')", commit_info.commit_unix)
        }
    };
    let sql = format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) VALUES ('{}', '{}', '{}', '{}', '{}', {}) \
ON CONFLICT (commit_sha) DO UPDATE SET repo_id = EXCLUDED.repo_id, author_name = EXCLUDED.author_name, author_email = EXCLUDED.author_email, commit_message = EXCLUDED.commit_message, committed_at = EXCLUDED.committed_at",
        esc_pg(&commit_info.commit_sha),
        esc_pg(&cfg.repo.repo_id),
        esc_pg(&commit_info.author_name),
        esc_pg(&commit_info.author_email),
        esc_pg(if commit_info.subject.is_empty() {
            &cp.checkpoint_id
        } else {
            &commit_info.subject
        }),
        committed_at_sql,
    );

    relational.exec(&sql).await
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
    let artefact_rows =
        crate::host::devql::checkpoint_provenance::collect_checkpoint_artefact_provenance_rows(
            &cfg.repo_root,
            context,
            &file_rows,
        )?;

    let mut sqlite_statements = Vec::with_capacity(2 + file_rows.len() + artefact_rows.len());
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
                RelationalDialect::Sqlite,
            ),
        );
    }
    for row in &artefact_rows {
        sqlite_statements.push(
            crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                row,
                RelationalDialect::Sqlite,
            ),
        );
    }
    relational
        .exec_batch_transactional(&sqlite_statements)
        .await?;

    if relational.remote.is_some() {
        let mut postgres_statements = Vec::with_capacity(2 + file_rows.len() + artefact_rows.len());
        postgres_statements.push(
            crate::host::devql::checkpoint_provenance::delete_checkpoint_artefact_rows_sql(
                &cfg.repo.repo_id,
                &cp.checkpoint_id,
            ),
        );
        postgres_statements.push(
            crate::host::devql::checkpoint_provenance::delete_checkpoint_file_rows_sql(
                &cfg.repo.repo_id,
                &cp.checkpoint_id,
            ),
        );
        for row in &file_rows {
            postgres_statements.push(
                crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_file_row_sql(
                    row,
                    RelationalDialect::Postgres,
                ),
            );
        }
        for row in &artefact_rows {
            postgres_statements.push(
                crate::host::devql::checkpoint_provenance::build_upsert_checkpoint_artefact_row_sql(
                    row,
                    RelationalDialect::Postgres,
                ),
            );
        }
        relational
            .exec_remote_batch_transactional(&postgres_statements)
            .await?;
    }

    Ok(file_rows.len())
}
