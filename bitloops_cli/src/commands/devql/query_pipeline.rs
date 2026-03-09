fn parse_devql_query(query: &str) -> Result<ParsedDevqlQuery> {
    let mut parsed = ParsedDevqlQuery {
        limit: 100,
        ..Default::default()
    };

    let stages = query
        .split("->")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    if stages.is_empty() {
        bail!("empty DevQL query")
    }

    for stage in stages {
        if let Some(inner) = stage
            .strip_prefix("repo(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.repo = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("asOf(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            if let Some(commit) = args.get("commit") {
                parsed.as_of = Some(AsOfSelector::Commit(commit.clone()));
            } else if let Some(reference) = args.get("ref") {
                parsed.as_of = Some(AsOfSelector::Ref(reference.clone()));
            } else {
                bail!("asOf(...) requires `commit:` or `ref:`")
            }
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("file(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.file = Some(parse_single_quoted_or_double(inner)?);
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("files(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.files_path = args.get("path").cloned();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("artefacts(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_artefacts_stage = true;
            parsed.artefacts.kind = args.get("kind").cloned();
            parsed.artefacts.agent = args.get("agent").cloned();
            parsed.artefacts.since = args.get("since").cloned();
            if let Some(lines) = args.get("lines") {
                parsed.artefacts.lines = Some(parse_lines_range(lines)?);
            }
            continue;
        }

        if stage == "artefacts()" {
            parsed.has_artefacts_stage = true;
            continue;
        }

        if stage == "chatHistory()" {
            parsed.has_chat_history_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("checkpoints(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_checkpoints_stage = true;
            parsed.checkpoints.agent = args.get("agent").cloned();
            parsed.checkpoints.since = args.get("since").cloned();
            continue;
        }

        if stage == "checkpoints()" {
            parsed.has_checkpoints_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("telemetry(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let args = parse_named_args(inner)?;
            parsed.has_telemetry_stage = true;
            parsed.telemetry.event_type = args.get("event_type").cloned();
            parsed.telemetry.agent = args.get("agent").cloned();
            parsed.telemetry.since = args.get("since").cloned();
            continue;
        }

        if stage == "telemetry()" {
            parsed.has_telemetry_stage = true;
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("select(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.select_fields = inner
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }

        if let Some(inner) = stage
            .strip_prefix("limit(")
            .and_then(|s| s.strip_suffix(')'))
        {
            parsed.limit = inner
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow!("invalid limit value: {inner}"))?;
            continue;
        }

        bail!("unsupported DevQL stage: {stage}")
    }

    Ok(parsed)
}

fn parse_named_args(input: &str) -> Result<BTreeMap<String, String>> {
    let mut args = BTreeMap::new();
    if input.trim().is_empty() {
        return Ok(args);
    }

    let mut current = String::new();
    let mut pieces = Vec::new();
    let mut in_quotes = false;
    let mut quote_char = '\0';

    for ch in input.chars() {
        match ch {
            '\'' | '"' => {
                if in_quotes && ch == quote_char {
                    in_quotes = false;
                    quote_char = '\0';
                } else if !in_quotes {
                    in_quotes = true;
                    quote_char = ch;
                }
                current.push(ch);
            }
            ',' if !in_quotes => {
                pieces.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }

    for piece in pieces {
        let Some((key, value)) = piece.split_once(':') else {
            bail!("invalid argument segment: {piece}")
        };
        let key = key.trim().to_string();
        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_single_quoted_or_double(value)?
        } else {
            value.to_string()
        };
        args.insert(key, value);
    }

    Ok(args)
}

fn parse_single_quoted_or_double(input: &str) -> Result<String> {
    let s = input.trim();
    if s.len() < 2 {
        bail!("expected quoted string, got: {input}")
    }

    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        return Ok(s[1..s.len() - 1].to_string());
    }

    bail!("expected quoted string, got: {input}")
}

fn parse_lines_range(lines: &str) -> Result<(i32, i32)> {
    let Some((start, end)) = lines.split_once("..") else {
        bail!("invalid lines range: {lines}")
    };
    let start = start
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line start: {start}"))?;
    let end = end
        .trim()
        .parse::<i32>()
        .map_err(|_| anyhow!("invalid line end: {end}"))?;
    if start <= 0 || end <= 0 || end < start {
        bail!("invalid lines range: {lines}")
    }
    Ok((start, end))
}

async fn execute_devql_query(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    pg_client: Option<&tokio_postgres::Client>,
) -> Result<Vec<Value>> {
    if (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
        && (parsed.file.is_some() || parsed.files_path.is_some() || parsed.has_artefacts_stage)
    {
        bail!(
            "MVP limitation: telemetry/checkpoints stages cannot be combined with artefact traversal in one query"
        )
    }

    if parsed.has_chat_history_stage && !parsed.has_artefacts_stage {
        bail!("chatHistory() requires an artefacts() stage in the query");
    }

    if parsed.has_chat_history_stage && (parsed.has_checkpoints_stage || parsed.has_telemetry_stage)
    {
        bail!("chatHistory() cannot be combined with checkpoints()/telemetry() stages");
    }

    if parsed.has_checkpoints_stage || parsed.has_telemetry_stage {
        return execute_clickhouse_pipeline(cfg, parsed).await;
    }

    let pg_client = pg_client.ok_or_else(|| anyhow!("Postgres client is required"))?;
    execute_postgres_pipeline(cfg, parsed, pg_client).await
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

async fn execute_postgres_pipeline(
    cfg: &DevqlConfig,
    parsed: &ParsedDevqlQuery,
    pg_client: &tokio_postgres::Client,
) -> Result<Vec<Value>> {
    let _ = cfg.require_pg_dsn()?;
    let repo_id = resolve_repo_id_for_query(cfg, parsed.repo.as_deref());

    let mut where_clauses = vec![format!("a.repo_id = '{}'", esc_pg(&repo_id))];

    if let Some(kind) = parsed.artefacts.kind.as_deref() {
        where_clauses.push(format!("a.canonical_kind = '{}'", esc_pg(kind)));
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
                    esc_pg(&repo_id),
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
        let blob_shas = blob_shas_changed_in_events(
            cfg,
            pg_client,
            &repo_id,
            parsed.artefacts.agent.as_deref(),
            parsed.artefacts.since.as_deref(),
        )
        .await?;
        if blob_shas.is_empty() {
            return Ok(vec![]);
        }
        where_clauses.push(format!(
            "a.blob_sha IN ({})",
            sql_string_list_pg(&blob_shas)
        ));
    }

    let sql = format!(
        "SELECT a.artefact_id, a.path, a.canonical_kind, a.language, a.start_line, a.end_line, a.start_byte, a.end_byte, a.signature, a.blob_sha, a.symbol_fqn, a.content_hash, a.created_at \
FROM artefacts a \
WHERE {} \
ORDER BY a.path, a.start_line \
LIMIT {}",
        where_clauses.join(" AND "),
        parsed.limit.max(1)
    );

    let rows = pg_query_rows(pg_client, &sql).await?;
    if parsed.has_chat_history_stage {
        return attach_chat_history_to_artefacts(cfg, pg_client, &repo_id, rows).await;
    }
    Ok(rows)
}

fn resolve_commit_selector(cfg: &DevqlConfig, parsed: &ParsedDevqlQuery) -> Result<Option<String>> {
    let Some(as_of) = parsed.as_of.as_ref() else {
        return Ok(None);
    };

    match as_of {
        AsOfSelector::Commit(commit) => Ok(Some(commit.clone())),
        AsOfSelector::Ref(reference) => {
            let commit = run_git(&cfg.repo_root, &["rev-parse", reference])
                .with_context(|| format!("resolving git ref `{reference}`"))?;
            Ok(Some(commit.trim().to_string()))
        }
    }
}

async fn blob_shas_changed_in_events(
    cfg: &DevqlConfig,
    pg_client: &tokio_postgres::Client,
    repo_id: &str,
    agent: Option<&str>,
    since: Option<&str>,
) -> Result<Vec<String>> {
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
    let commit_shas = data
        .as_array()
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
        .collect::<Vec<_>>();

    if commit_shas.is_empty() {
        return Ok(vec![]);
    }

    let sql = format!(
        "SELECT DISTINCT fs.blob_sha FROM file_state fs WHERE fs.repo_id = '{}' AND fs.commit_sha IN ({})",
        esc_pg(repo_id),
        sql_string_list_pg(&commit_shas),
    );
    let rows = pg_query_rows(pg_client, &sql).await?;
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
    pg_client: &tokio_postgres::Client,
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
                commit_shas_for_artefact_blob(pg_client, repo_id, &path, &blob_sha).await?;
            let events = checkpoint_events_for_commits(cfg, repo_id, &path, &commit_shas).await?;
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
    pg_client: &tokio_postgres::Client,
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
    let rows = pg_query_rows(pg_client, &sql).await?;
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
    repo_id: &str,
    path: &str,
    commit_shas: &[String],
) -> Result<Vec<Value>> {
    if commit_shas.is_empty() {
        return Ok(vec![]);
    }

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

fn extract_chat_messages_from_transcript(transcript: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    for line in transcript.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let Some(text) = extract_message_text(&value) else {
            continue;
        };
        let role = extract_message_role(&value).unwrap_or_else(|| "unknown".to_string());
        messages.push(json!({
            "role": role,
            "text": text,
        }));
    }
    messages
}

fn extract_message_role(value: &Value) -> Option<String> {
    value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/role").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn extract_message_text(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(flatten_text_value)
        .or_else(|| value.get("content").and_then(flatten_text_value))
        .or_else(|| value.get("text").and_then(flatten_text_value))
}

fn flatten_text_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = flatten_text_value(item) {
                    parts.push(text);
                }
            }

            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        Value::Object(map) => map
            .get("text")
            .and_then(flatten_text_value)
            .or_else(|| map.get("content").and_then(flatten_text_value))
            .or_else(|| map.get("input").and_then(flatten_text_value)),
        _ => None,
    }
}

fn resolve_repo_id_for_query(cfg: &DevqlConfig, requested_repo: Option<&str>) -> String {
    let Some(repo) = requested_repo else {
        return cfg.repo.repo_id.clone();
    };

    let normalized = repo.trim();
    if normalized.is_empty() {
        return cfg.repo.repo_id.clone();
    }

    let local_candidates = [
        cfg.repo.name.as_str(),
        cfg.repo.identity.as_str(),
        &format!("{}/{}", cfg.repo.organization, cfg.repo.name),
    ];

    if local_candidates
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(normalized))
    {
        return cfg.repo.repo_id.clone();
    }

    deterministic_uuid(&format!("repo://{normalized}"))
}

fn sql_string_list_ch(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn sql_string_list_pg(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn project_rows(rows: Vec<Value>, fields: &[String]) -> Vec<Value> {
    if fields.is_empty() {
        return rows;
    }

    if fields.len() == 1 && fields[0].trim() == "count()" {
        return vec![json!({ "count": rows.len() })];
    }

    let mut projected = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(obj) = row.as_object() {
            let mut out = Map::new();
            for field in fields {
                if field.trim() == "count()" {
                    continue;
                }
                if let Some(value) = lookup_nested_field(obj, field.trim()) {
                    out.insert(field.trim().to_string(), value.clone());
                } else {
                    out.insert(field.trim().to_string(), Value::Null);
                }
            }
            projected.push(Value::Object(out));
        } else {
            projected.push(row);
        }
    }
    projected
}

fn lookup_nested_field<'a>(obj: &'a Map<String, Value>, field: &str) -> Option<&'a Value> {
    if !field.contains('.') {
        return obj.get(field);
    }

    let mut current: Option<&Value> = None;
    for (index, part) in field.split('.').enumerate() {
        if index == 0 {
            current = obj.get(part);
        } else {
            current = current.and_then(Value::as_object).and_then(|m| m.get(part));
        }
    }
    current
}
