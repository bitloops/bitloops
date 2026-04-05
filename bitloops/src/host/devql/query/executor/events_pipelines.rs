use super::*;

pub(crate) async fn execute_clickhouse_pipeline(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());

    if parsed.has_checkpoints_stage {
        return execute_committed_checkpoints_stage(cfg, parsed, &repo_id);
    }

    let mut conditions = vec![format!("repo_id = '{}'", esc_ch(&repo_id))];
    if let Some(event_type) = parsed.telemetry.event_type.as_deref() {
        conditions.push(format!("event_type = '{}'", esc_ch(event_type)));
    }
    if let Some(agent) = parsed.telemetry.agent.as_deref() {
        conditions.push(format!("agent = '{}'", esc_ch(agent)));
    }
    if let Some(since) = parsed.telemetry.since.as_deref() {
        conditions.push(format!(
            "event_time >= parseDateTime64BestEffortOrZero('{}')",
            esc_ch(since)
        ));
    }

    let sql = format!(
        "SELECT event_time, event_type, checkpoint_id, session_id, agent, commit_sha, branch, strategy, files_touched, payload FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT {} FORMAT JSON",
        conditions.join(" AND "),
        parsed.limit.max(1)
    );

    let data = clickhouse_query_data(cfg, &sql).await?;
    Ok(data.as_array().cloned().unwrap_or_default())
}

pub(crate) fn normalise_duckdb_event_row(row: Value) -> Value {
    let Some(mut obj) = row.as_object().cloned() else {
        return row;
    };

    if let Some(files_touched_raw) = obj.get("files_touched").and_then(Value::as_str)
        && let Ok(files_touched) = serde_json::from_str::<Value>(files_touched_raw)
    {
        obj.insert("files_touched".to_string(), files_touched);
    }

    if let Some(payload_raw) = obj.get("payload").and_then(Value::as_str)
        && let Ok(payload) = serde_json::from_str::<Value>(payload_raw)
    {
        obj.insert("payload".to_string(), payload);
    }

    Value::Object(obj)
}

pub(crate) async fn execute_duckdb_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());
    let duckdb_path = events_cfg.duckdb_path_or_default();

    if parsed.has_checkpoints_stage {
        return execute_committed_checkpoints_stage(cfg, parsed, &repo_id);
    }

    let mut conditions = vec![format!("repo_id = '{}'", esc_pg(&repo_id))];
    if let Some(event_type) = parsed.telemetry.event_type.as_deref() {
        conditions.push(format!("event_type = '{}'", esc_pg(event_type)));
    }
    if let Some(agent) = parsed.telemetry.agent.as_deref() {
        conditions.push(format!("agent = '{}'", esc_pg(agent)));
    }
    if let Some(since) = parsed.telemetry.since.as_deref() {
        conditions.push(format!("event_time >= '{}'", esc_pg(since)));
    }

    let sql = format!(
        "SELECT event_time, event_type, checkpoint_id, session_id, agent, commit_sha, branch, strategy, files_touched, payload FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT {}",
        conditions.join(" AND "),
        parsed.limit.max(1)
    );

    let rows = duckdb_query_rows_path(&duckdb_path, &sql).await?;
    Ok(rows
        .into_iter()
        .map(normalise_duckdb_event_row)
        .collect::<Vec<_>>())
}

fn execute_committed_checkpoints_stage(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let mut checkpoints = list_committed(&cfg.repo_root)?;
    let commit_map = collect_checkpoint_commit_map(&cfg.repo_root)?;
    checkpoints.retain(|checkpoint| {
        if repo_id != cfg.repo.repo_id {
            return false;
        }
        if let Some(agent) = parsed.checkpoints.agent.as_deref()
            && checkpoint.agent != agent
        {
            return false;
        }
        if let Some(since) = parsed.checkpoints.since.as_deref()
            && !checkpoint.created_at.is_empty()
            && checkpoint.created_at.as_str() < since
        {
            return false;
        }
        true
    });

    checkpoints.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.checkpoint_id.cmp(&left.checkpoint_id))
    });

    Ok(checkpoints
        .into_iter()
        .take(parsed.limit.max(1))
        .map(|checkpoint| {
            let commit_sha = commit_map
                .get(&checkpoint.checkpoint_id)
                .map(|info| info.commit_sha.clone())
                .unwrap_or_default();
            serde_json::json!({
                "checkpoint_id": checkpoint.checkpoint_id,
                "created_at": checkpoint.created_at,
                "agent": checkpoint.agent,
                "commit_sha": commit_sha,
                "branch": checkpoint.branch,
                "strategy": checkpoint.strategy,
                "files_touched": checkpoint.files_touched,
            })
        })
        .collect())
}
