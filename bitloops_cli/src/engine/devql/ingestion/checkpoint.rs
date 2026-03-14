// Checkpoint and commit row persistence: mapping, event insertion, upserts.

async fn ensure_repository_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
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
    postgres_exec(pg_client, &sql).await
}

fn default_branch_name(repo_root: &Path) -> String {
    run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_else(|_| "main".to_string())
}

fn collect_checkpoint_commit_map(
    repo_root: &Path,
) -> Result<HashMap<String, CheckpointCommitInfo>> {
    match collect_checkpoint_commit_map_from_db(repo_root) {
        Ok(map) if !map.is_empty() => return Ok(map),
        Ok(_) => {}
        Err(err) => {
            log::debug!(
                "devql ingest: failed to read commit_checkpoints mapping (falling back to trailers): {:#}",
                err
            );
        }
    }

    collect_checkpoint_commit_map_from_trailers(repo_root)
}

fn collect_checkpoint_commit_map_from_db(
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
                        && info.commit_sha > existing.commit_sha)
            }
        };
        if should_replace {
            out.insert(checkpoint_id, info);
        }
    }

    Ok(out)
}

fn checkpoint_commit_info_from_sha(
    repo_root: &Path,
    commit_sha: &str,
) -> Option<CheckpointCommitInfo> {
    if commit_sha.trim().is_empty() {
        return None;
    }

    let raw = run_git(
        repo_root,
        &[
            "show",
            "-s",
            "--format=%ct%x1f%an%x1f%ae%x1f%s",
            commit_sha,
        ],
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

fn collect_checkpoint_commit_map_from_trailers(
    repo_root: &Path,
) -> Result<HashMap<String, CheckpointCommitInfo>> {
    let fmt = format!(
        "%H%x1f%ct%x1f%an%x1f%ae%x1f%s%x1f%(trailers:key={CHECKPOINT_TRAILER_KEY},valueonly=true,separator=%x00)%x1e"
    );
    let raw = run_git(
        repo_root,
        &[
            "log",
            "--all",
            "--date-order",
            &format!("--format={fmt}"),
            "--max-count=50000",
            "--no-color",
        ],
    )
    .unwrap_or_default();

    let mut out = HashMap::new();
    for record in raw.split('\u{1e}') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut parts = record.split('\u{1f}');
        let commit_sha = parts.next().unwrap_or_default().trim().to_string();
        let commit_unix = parts
            .next()
            .unwrap_or_default()
            .trim()
            .parse::<i64>()
            .unwrap_or(0);
        let author_name = parts.next().unwrap_or_default().trim().to_string();
        let author_email = parts.next().unwrap_or_default().trim().to_string();
        let subject = parts.next().unwrap_or_default().trim().to_string();
        let checkpoints = parts.next().unwrap_or_default();

        if commit_sha.is_empty() {
            continue;
        }

        for cp in checkpoints.split('\x00').map(str::trim) {
            if !is_valid_checkpoint_id(cp) {
                continue;
            }
            out.entry(cp.to_string())
                .or_insert_with(|| CheckpointCommitInfo {
                    commit_sha: commit_sha.clone(),
                    commit_unix,
                    author_name: author_name.clone(),
                    author_email: author_email.clone(),
                    subject: subject.clone(),
                });
        }
    }

    Ok(out)
}

async fn fetch_existing_checkpoint_event_ids(cfg: &DevqlConfig) -> Result<HashSet<String>> {
    let sql = format!(
        "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}' FORMAT JSON",
        esc_ch(&cfg.repo.repo_id)
    );

    let mut out = HashSet::new();
    let data = clickhouse_query_data(cfg, &sql).await?;
    if let Some(rows) = data.as_array() {
        for row in rows {
            if let Some(id) = row.get("event_id").and_then(Value::as_str) {
                out.insert(id.to_string());
            }
        }
    }
    Ok(out)
}

async fn insert_checkpoint_event(
    cfg: &DevqlConfig,
    cp: &CommittedInfo,
    event_id: &str,
    commit_info: Option<&CheckpointCommitInfo>,
) -> Result<()> {
    let event_time_expr = if !cp.created_at.trim().is_empty() {
        format!(
            "coalesce(parseDateTime64BestEffortOrNull('{}'), now64(3))",
            esc_ch(cp.created_at.trim())
        )
    } else if let Some(info) = commit_info {
        format!("toDateTime64({}, 3, 'UTC')", info.commit_unix)
    } else {
        "now64(3)".to_string()
    };

    let commit_sha = commit_info
        .map(|info| info.commit_sha.as_str())
        .unwrap_or_default();

    let payload = json!({
        "checkpoints_count": cp.checkpoints_count,
        "session_count": cp.session_count,
        "token_usage": cp.token_usage,
    });

    let files_touched = format_ch_array(&cp.files_touched);
    let sql = format!(
        "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched, payload) \
VALUES ('{}', {}, '{}', '{}', '{}', '{}', '{}', 'checkpoint_committed', '{}', '{}', {}, '{}')",
        esc_ch(event_id),
        event_time_expr,
        esc_ch(&cfg.repo.repo_id),
        esc_ch(&cp.checkpoint_id),
        esc_ch(&cp.session_id),
        esc_ch(commit_sha),
        esc_ch(&cp.branch),
        esc_ch(&cp.agent),
        esc_ch(&cp.strategy),
        files_touched,
        esc_ch(&serde_json::to_string(&payload)?),
    );

    clickhouse_exec(cfg, &sql).await.map(|_| ())
}

async fn upsert_commit_row(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    cp: &CommittedInfo,
    commit_info: &CheckpointCommitInfo,
) -> Result<()> {
    let sql = format!(
        "INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at) VALUES ('{}', '{}', '{}', '{}', '{}', to_timestamp({})) \
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
        commit_info.commit_unix,
    );

    postgres_exec(pg_client, &sql).await
}
