async fn blob_shas_changed_in_events(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    relational: &RelationalStorage,
    repo_id: &str,
    agent: Option<&str>,
    since: Option<&str>,
) -> Result<Vec<String>> {
    let commit_shas = match events_cfg.provider {
        EventsProvider::ClickHouse => {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_ch(repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                "commit_sha != ''".to_string(),
            ];

            if let Some(agent) = agent {
                conditions.push(format!("agent = '{}'", esc_ch(agent)));
            }
            if let Some(since) = since {
                conditions.push(format!(
                    "event_time >= parseDateTime64BestEffortOrZero('{}')",
                    esc_ch(since)
                ));
            }

            let sql = format!(
                "SELECT DISTINCT commit_sha FROM checkpoint_events WHERE {} LIMIT 20000 FORMAT JSON",
                conditions.join(" AND ")
            );
            let data = clickhouse_query_data(cfg, &sql).await?;
            data.as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|row| {
                    row.get("commit_sha")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        }
        EventsProvider::DuckDb => {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_pg(repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                "commit_sha <> ''".to_string(),
            ];
            if let Some(agent) = agent {
                conditions.push(format!("agent = '{}'", esc_pg(agent)));
            }
            if let Some(since) = since {
                conditions.push(format!("event_time >= '{}'", esc_pg(since)));
            }
            let sql = format!(
                "SELECT DISTINCT commit_sha FROM checkpoint_events WHERE {} LIMIT 20000",
                conditions.join(" AND ")
            );
            let rows = duckdb_query_rows_path(&events_cfg.duckdb_path_or_default(), &sql).await?;
            rows.into_iter()
                .filter_map(|row| {
                    row.get("commit_sha")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        }
    };

    if commit_shas.is_empty() {
        return Ok(vec![]);
    }

    let sql = format!(
        "SELECT DISTINCT fs.blob_sha FROM file_state fs WHERE fs.repo_id = '{}' AND fs.commit_sha IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(&commit_shas),
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("blob_sha")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect())
}

async fn attach_chat_history_to_artefacts(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    relational: &RelationalStorage,
    repo_id: &str,
    rows: Vec<Value>,
) -> Result<Vec<Value>> {
    let mut out = Vec::with_capacity(rows.len());
    let mut artefact_history_cache: HashMap<(String, String), Vec<Value>> = HashMap::new();
    let mut session_chat_cache: HashMap<(String, String), Option<Value>> = HashMap::new();

    for row in rows {
        let Some(obj) = row.as_object() else {
            out.push(row);
            continue;
        };

        let path = obj
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let blob_sha = obj
            .get("blob_sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        let history = if path.is_empty() || blob_sha.is_empty() {
            vec![]
        } else if let Some(cached) = artefact_history_cache.get(&(path.clone(), blob_sha.clone())) {
            cached.clone()
        } else {
            let commit_shas =
                commit_shas_for_artefact_blob(relational, repo_id, &path, &blob_sha).await?;
            let events =
                checkpoint_events_for_commits(cfg, events_cfg, repo_id, &path, &commit_shas)
                    .await?;
            let mut history_entries = Vec::with_capacity(events.len());

            for event in events {
                let mut event_obj = event.as_object().cloned().unwrap_or_default();
                let checkpoint_id = event_obj
                    .get("checkpoint_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let session_id = event_obj
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();

                if !checkpoint_id.is_empty()
                    && !session_id.is_empty()
                    && let Some(chat) = session_chat_payload(
                        cfg,
                        checkpoint_id,
                        session_id,
                        &mut session_chat_cache,
                    )
                {
                    event_obj.insert("chat".to_string(), chat);
                }

                history_entries.push(Value::Object(event_obj));
            }

            artefact_history_cache
                .insert((path.clone(), blob_sha.clone()), history_entries.clone());
            history_entries
        };

        let mut enriched = obj.clone();
        enriched.insert("chat_history".to_string(), Value::Array(history));
        out.push(Value::Object(enriched));
    }

    Ok(out)
}

async fn commit_shas_for_artefact_blob(
    relational: &RelationalStorage,
    repo_id: &str,
    path: &str,
    blob_sha: &str,
) -> Result<Vec<String>> {
    let path_candidates = build_path_candidates(path);
    let path_clause = sql_path_candidates_clause("fs.path", &path_candidates);
    let sql = format!(
        "SELECT DISTINCT fs.commit_sha FROM file_state fs WHERE fs.repo_id = '{}' AND fs.blob_sha = '{}' AND ({}) LIMIT 2000",
        esc_pg(repo_id),
        esc_pg(blob_sha),
        path_clause
    );
    let rows = relational.query_rows(&sql).await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            row.get("commit_sha")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect())
}

async fn checkpoint_events_for_commits(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    repo_id: &str,
    path: &str,
    commit_shas: &[String],
) -> Result<Vec<Value>> {
    if commit_shas.is_empty() {
        return Ok(vec![]);
    }

    match events_cfg.provider {
        EventsProvider::ClickHouse => {
            let path_candidates = build_path_candidates(path);
            let path_has_clause = if path_candidates.is_empty() {
                None
            } else {
                Some(
                    path_candidates
                        .iter()
                        .map(|candidate| format!("has(files_touched, '{}')", esc_ch(candidate)))
                        .collect::<Vec<_>>()
                        .join(" OR "),
                )
            };

            let mut conditions = vec![
                format!("repo_id = '{}'", esc_ch(repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                format!("commit_sha IN ({})", sql_string_list_ch(commit_shas)),
            ];
            if let Some(path_has_clause) = path_has_clause {
                conditions.push(format!("({path_has_clause})"));
            }

            let sql = format!(
                "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT 200 FORMAT JSON",
                conditions.join(" AND ")
            );
            let data = clickhouse_query_data(cfg, &sql).await?;
            Ok(data.as_array().cloned().unwrap_or_default())
        }
        EventsProvider::DuckDb => {
            let path_candidates = build_path_candidates(path);
            let path_has_clause = if path_candidates.is_empty() {
                None
            } else {
                Some(
                    path_candidates
                        .iter()
                        .map(|candidate| {
                            format!(
                                "files_touched LIKE '%\"{}\"%'",
                                esc_pg(candidate).replace('%', "\\%")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" OR "),
                )
            };

            let mut conditions = vec![
                format!("repo_id = '{}'", esc_pg(repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                format!("commit_sha IN ({})", sql_string_list_pg(commit_shas)),
            ];
            if let Some(path_has_clause) = path_has_clause {
                conditions.push(format!("({path_has_clause})"));
            }

            let sql = format!(
                "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy, files_touched, payload FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT 200",
                conditions.join(" AND ")
            );
            let rows = duckdb_query_rows_path(&events_cfg.duckdb_path_or_default(), &sql).await?;
            Ok(rows
                .into_iter()
                .map(normalise_duckdb_event_row)
                .collect::<Vec<_>>())
        }
    }
}

fn session_chat_payload(
    cfg: &DevqlConfig,
    checkpoint_id: &str,
    session_id: &str,
    cache: &mut HashMap<(String, String), Option<Value>>,
) -> Option<Value> {
    let key = (checkpoint_id.to_string(), session_id.to_string());
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }

    let mut resolved: Option<Value> = None;
    if let Ok(Some(summary)) = read_committed(&cfg.repo_root, checkpoint_id) {
        for idx in 0..summary.sessions.len() {
            let Ok(content) = read_session_content(&cfg.repo_root, checkpoint_id, idx) else {
                continue;
            };
            let current_session_id = content
                .metadata
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if current_session_id != session_id {
                continue;
            }

            let mut obj = Map::new();
            if let Some(created_at) = content.metadata.get("created_at").and_then(Value::as_str) {
                obj.insert(
                    "created_at".to_string(),
                    Value::String(created_at.to_string()),
                );
            }

            let prompts = content.prompts.trim();
            if !prompts.is_empty() {
                obj.insert("prompts".to_string(), Value::String(prompts.to_string()));
            }

            let messages = extract_chat_messages_from_transcript(&content.transcript);
            if messages.is_empty() {
                let transcript = content.transcript.trim();
                if !transcript.is_empty() {
                    obj.insert(
                        "transcript".to_string(),
                        Value::String(transcript.to_string()),
                    );
                }
            } else {
                obj.insert("messages".to_string(), Value::Array(messages));
            }

            if let Some(summary_value) = content.metadata.get("summary") {
                obj.insert("summary".to_string(), summary_value.clone());
            }

            resolved = Some(Value::Object(obj));
            break;
        }
    }

    cache.insert(key, resolved.clone());
    resolved
}

async fn execute_registered_stages(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    mut rows: Vec<Value>,
) -> Result<Vec<Value>> {
    if parsed.registered_stages.is_empty() {
        return Ok(rows);
    }

    let mut host = build_capability_host(&cfg.repo_root, cfg.repo.clone())?;
    for stage in &parsed.registered_stages {
        let capability_id = if host.has_stage("knowledge", &stage.stage_name) {
            "knowledge"
        } else {
            bail!("unsupported DevQL stage: {}()", stage.stage_name);
        };

        let response = host
            .invoke_stage(
                capability_id,
                &stage.stage_name,
                json!({
                    "input_rows": rows,
                    "args": stage.args,
                    "limit": parsed.limit.max(1),
                }),
            )
            .await?;
        rows = match response.payload {
            Value::Array(array) => array,
            value => vec![value],
        };
    }

    Ok(rows)
}
