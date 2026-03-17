async fn execute_devql_query(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    events_cfg: &EventsBackendConfig,
    relational: Option<&RelationalStorage>,
) -> Result<Vec<Value>> {
    if (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
        && (parsed.file.is_some() || parsed.files_path.is_some() || parsed.has_artefacts_stage)
    {
        log_devql_validation_failure(
            parsed,
            "telemetry_or_checkpoints_with_artefacts",
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query",
        );
        bail!(
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query"
        )
    }

    if parsed.has_chat_history_stage && !parsed.has_artefacts_stage {
        log_devql_validation_failure(
            parsed,
            "chat_history_requires_artefacts",
            "chatHistory() requires an artefacts() stage in the query",
        );
        bail!("chatHistory() requires an artefacts() stage in the query");
    }

    if parsed.has_deps_stage && parsed.has_chat_history_stage {
        log_devql_validation_failure(
            parsed,
            "deps_with_chat_history",
            "deps() cannot be combined with chatHistory() stage",
        );
        bail!("deps() cannot be combined with chatHistory() stage");
    }

    if parsed.has_chat_history_stage && (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
    {
        log_devql_validation_failure(
            parsed,
            "chat_history_with_telemetry_or_checkpoints",
            "chatHistory() cannot be combined with checkpoints()/telemetry() stages",
        );
        bail!("chatHistory() cannot be combined with checkpoints()/telemetry() stages");
    }

    if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        return match events_cfg.provider {
            EventsProvider::ClickHouse => execute_clickhouse_pipeline(cfg, parsed).await,
            EventsProvider::DuckDb => execute_duckdb_pipeline(cfg, events_cfg, parsed).await,
        };
    }

    let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
    execute_relational_pipeline(cfg, events_cfg, parsed, relational).await
}

fn log_devql_validation_failure(parsed: &ParsedDevqlQuery, rule: &str, reason: &str) {
    crate::engine::logging::warn(
        &crate::engine::logging::with_component(crate::engine::logging::background(), "devql"),
        "devql query validation failed",
        &[
            crate::engine::logging::string_attr("rule", rule),
            crate::engine::logging::string_attr("reason", reason),
            crate::engine::logging::bool_attr("has_deps_stage", parsed.has_deps_stage),
            crate::engine::logging::bool_attr(
                "has_chat_history_stage",
                parsed.has_chat_history_stage,
            ),
            crate::engine::logging::bool_attr(
                "has_checkpoints_stage",
                parsed.has_checkpoints_stage,
            ),
            crate::engine::logging::bool_attr("has_telemetry_stage", parsed.has_telemetry_stage),
        ],
    );
}

async fn execute_clickhouse_pipeline(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());

    if parsed.has_checkpoints_stage {
        let mut conditions = vec![
            format!("repo_id = '{}'", esc_ch(&repo_id)),
            "event_type = 'checkpoint_committed'".to_string(),
        ];
        if let Some(agent) = parsed.checkpoints.agent.as_deref() {
            conditions.push(format!("agent = '{}'", esc_ch(agent)));
        }
        if let Some(since) = parsed.checkpoints.since.as_deref() {
            conditions.push(format!(
                "event_time >= parseDateTime64BestEffortOrZero('{}')",
                esc_ch(since)
            ));
        }

        let sql = format!(
            "SELECT checkpoint_id, max(event_time) AS created_at, anyLast(agent) AS agent, anyLast(commit_sha) AS commit_sha, anyLast(branch) AS branch, anyLast(strategy) AS strategy, anyLast(files_touched) AS files_touched FROM checkpoint_events WHERE {} GROUP BY checkpoint_id ORDER BY created_at DESC LIMIT {} FORMAT JSON",
            conditions.join(" AND "),
            parsed.limit.max(1)
        );

        let data = clickhouse_query_data(cfg, &sql).await?;
        return Ok(data.as_array().cloned().unwrap_or_default());
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

fn normalise_duckdb_event_row(row: Value) -> Value {
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

fn normalise_relational_result_row(row: Value) -> Value {
    let Some(mut obj) = row.as_object().cloned() else {
        return row;
    };

    for key in ["modifiers", "metadata"] {
        if let Some(raw) = obj.get(key).and_then(Value::as_str)
            && let Ok(parsed) = serde_json::from_str::<Value>(raw)
        {
            obj.insert(key.to_string(), parsed);
        }
    }

    if let Some(edge_kind) = obj.get("edge_kind").and_then(Value::as_str)
        && let Some(normalized) = normalise_edge_kind_value(edge_kind)
    {
        obj.insert("edge_kind".to_string(), Value::String(normalized.clone()));
        if let Some(metadata) = obj.get_mut("metadata") {
            normalise_edge_metadata(&normalized, metadata);
        }
    }

    Value::Object(obj)
}

async fn execute_duckdb_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());
    let duckdb_path = events_cfg.duckdb_path_or_default();

    if parsed.has_checkpoints_stage {
        let mut conditions = vec![
            format!("repo_id = '{}'", esc_pg(&repo_id)),
            "event_type = 'checkpoint_committed'".to_string(),
        ];
        if let Some(agent) = parsed.checkpoints.agent.as_deref() {
            conditions.push(format!("agent = '{}'", esc_pg(agent)));
        }
        if let Some(since) = parsed.checkpoints.since.as_deref() {
            conditions.push(format!("event_time >= '{}'", esc_pg(since)));
        }

        let sql = format!(
            "SELECT checkpoint_id, \
max(event_time) AS created_at, \
arg_max(agent, event_time) AS agent, \
arg_max(commit_sha, event_time) AS commit_sha, \
arg_max(branch, event_time) AS branch, \
arg_max(strategy, event_time) AS strategy, \
arg_max(files_touched, event_time) AS files_touched \
FROM checkpoint_events WHERE {} GROUP BY checkpoint_id ORDER BY created_at DESC LIMIT {}",
            conditions.join(" AND "),
            parsed.limit.max(1)
        );
        let rows = duckdb_query_rows_path(&duckdb_path, &sql).await?;
        return Ok(rows
            .into_iter()
            .map(normalise_duckdb_event_row)
            .collect::<Vec<_>>());
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

async fn execute_relational_pipeline(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
) -> Result<Vec<Value>> {
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());

    if parsed.has_deps_stage {
        return execute_relational_deps_pipeline(cfg, parsed, relational, &repo_id).await;
    }

    let sql =
        build_relational_artefacts_query(cfg, events_cfg, parsed, Some(relational), &repo_id)
            .await?;
    let rows = relational
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(normalise_relational_result_row)
        .collect::<Vec<_>>();
    if parsed.has_chat_history_stage {
        return attach_chat_history_to_artefacts(cfg, events_cfg, relational, &repo_id, rows)
            .await;
    }
    Ok(rows)
}

async fn build_relational_artefacts_query(
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    parsed: &ParsedDevqlQuery,
    relational: Option<&RelationalStorage>,
    repo_id: &str,
) -> Result<String> {
    let use_historical_tables = parsed.as_of.is_some();
    let artefacts_table = if use_historical_tables {
        "artefacts"
    } else {
        "artefacts_current"
    };
    let created_at_select = if use_historical_tables {
        "a.created_at"
    } else {
        "a.updated_at AS created_at"
    };

    let mut where_clauses = vec![format!("a.repo_id = '{}'", esc_pg(repo_id))];

    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        where_clauses.push(format!("a.canonical_kind = '{}'", esc_pg(kind)));
    }
    if let Some(symbol_fqn) = parsed.artefacts.symbol_fqn.as_deref() {
        where_clauses.push(format!("a.symbol_fqn = '{}'", esc_pg(symbol_fqn)));
    }

    if let Some((start, end)) = parsed.artefacts.lines {
        where_clauses.push(format!("a.start_line <= {end} AND a.end_line >= {start}"));
    }

    if let Some(path) = parsed.file.as_deref() {
        let path_candidates = build_path_candidates(path);
        if let Some(commit_sha) = resolve_commit_selector(cfg, parsed)? {
            let git_blob = path_candidates.iter().find_map(|candidate| {
                git_blob_sha_at_commit(&cfg.repo_root, &commit_sha, candidate)
            });

            if let Some(blob_sha) = git_blob {
                where_clauses.push(format!("a.blob_sha = '{}'", esc_pg(&blob_sha)));
            } else {
                where_clauses.push(format!(
                    "a.blob_sha = (SELECT blob_sha FROM file_state WHERE repo_id = '{}' AND commit_sha = '{}' AND ({}) LIMIT 1)",
                    esc_pg(repo_id),
                    esc_pg(&commit_sha),
                    sql_path_candidates_clause("path", &path_candidates),
                ));
            }
        } else {
            where_clauses.push(format!(
                "({})",
                sql_path_candidates_clause("a.path", &path_candidates)
            ));
        }
    }

    if let Some(glob) = parsed.files_path.as_deref() {
        let like = glob_to_sql_like(glob);
        where_clauses.push(format!("a.path LIKE '{}'", esc_pg(&like)));
    }

    if parsed.artefacts.agent.is_some() || parsed.artefacts.since.is_some() {
        let relational = relational.ok_or_else(|| anyhow!("relational storage is required"))?;
        let blob_shas = blob_shas_changed_in_events(
            cfg,
            events_cfg,
            relational,
            repo_id,
            parsed.artefacts.agent.as_deref(),
            parsed.artefacts.since.as_deref(),
        )
        .await?;
        if blob_shas.is_empty() {
            where_clauses.push("1 = 0".to_string());
        } else {
            where_clauses.push(format!(
                "a.blob_sha IN ({})",
                sql_string_list_pg(&blob_shas)
            ));
        }
    }

    let sql = format!(
        "SELECT a.artefact_id, a.path, a.canonical_kind, a.language_kind, a.language, a.start_line, a.end_line, a.start_byte, a.end_byte, a.signature, a.modifiers, a.docstring, a.blob_sha, a.symbol_fqn, a.content_hash, {} \
FROM {} a \
WHERE {} \
ORDER BY a.path, a.start_line \
LIMIT {}",
        created_at_select,
        artefacts_table,
        where_clauses.join(" AND "),
        parsed.limit.max(1)
    );

    Ok(sql)
}

async fn execute_relational_deps_pipeline(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    relational: &RelationalStorage,
    repo_id: &str,
) -> Result<Vec<Value>> {
    let sql = build_relational_deps_query(cfg, parsed, repo_id, relational.dialect())?;
    Ok(relational
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(normalise_relational_result_row)
        .collect::<Vec<_>>())
}

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
