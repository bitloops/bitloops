use super::git_history::git_default_branch_name;
use super::{DevqlGraphqlContext, GRAPHQL_GIT_SCAN_LIMIT};
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    ChatEntry, ChatRole, ClonesFilterInput, DateTimeScalar, SemanticClone,
};
use crate::host::checkpoints::strategy::manual_commit::{
    SessionContentView, read_session_content_by_id,
};
use crate::host::devql::{esc_ch, esc_pg, escape_like_pattern, sql_like_with_escape};
use anyhow::{Context, Result, anyhow, bail};
use async_graphql::types::Json;
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

impl DevqlGraphqlContext {
    pub(crate) async fn list_project_clones(
        &self,
        scope: &ResolverScope,
        filter: Option<&ClonesFilterInput>,
    ) -> Result<Vec<SemanticClone>> {
        let Some(project_path) = scope.project_path() else {
            return Ok(Vec::new());
        };

        let sql = build_project_clones_sql(
            self.repo_identity.repo_id.as_str(),
            &git_default_branch_name(self.repo_root.as_path()),
            project_path,
            filter,
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn list_artefact_clones(
        &self,
        artefact_id: &str,
        filter: Option<&ClonesFilterInput>,
        scope: &ResolverScope,
    ) -> Result<Vec<SemanticClone>> {
        let sql = build_artefact_clones_sql(
            self.repo_identity.repo_id.as_str(),
            &git_default_branch_name(self.repo_root.as_path()),
            artefact_id,
            scope.project_path(),
            filter,
        );
        let rows = self.query_devql_sqlite_rows(&sql).await?;
        rows.into_iter()
            .map(clone_from_row)
            .map(|result| result.map(|clone| clone.with_scope(scope.clone())))
            .collect()
    }

    pub(crate) async fn load_chat_history_by_paths(
        &self,
        paths: &[String],
        _scope: &ResolverScope,
    ) -> Result<HashMap<String, Vec<ChatEntry>>> {
        let unique_paths = dedup_paths(paths);
        if unique_paths.is_empty() {
            return Ok(HashMap::new());
        }

        let path_candidates = unique_paths
            .iter()
            .map(|path| (path.clone(), build_path_candidates(path)))
            .collect::<HashMap<_, _>>();
        let events = self.list_chat_history_events(&path_candidates).await?;

        let mut entries_by_path = unique_paths
            .iter()
            .cloned()
            .map(|path| (path, Vec::new()))
            .collect::<HashMap<_, _>>();
        let mut session_cache = HashMap::<(String, String), Vec<SessionMessageRecord>>::new();

        for event in events {
            let matching_paths = path_candidates
                .iter()
                .filter(|(_, candidates)| files_touched_matches(&event.files_touched, candidates))
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            if matching_paths.is_empty() {
                continue;
            }

            let cache_key = (event.checkpoint_id.clone(), event.session_id.clone());
            let session_messages = session_cache
                .entry(cache_key)
                .or_insert_with(|| {
                    load_session_messages(
                        self.repo_root.as_path(),
                        &event.checkpoint_id,
                        &event.session_id,
                        &event.event_time,
                    )
                })
                .clone();
            if session_messages.is_empty() {
                continue;
            }

            for path in matching_paths {
                let path_entries = entries_by_path.entry(path.clone()).or_default();
                let start_index = path_entries.len();

                for (message_index, message) in session_messages.iter().enumerate() {
                    let metadata =
                        build_chat_metadata(&event, message.raw_role.as_deref(), message_index);
                    path_entries.push(ChatEntry {
                        session_id: event.session_id.clone(),
                        agent: event.agent.clone(),
                        timestamp: message.timestamp.clone(),
                        role: message.role,
                        content: message.content.clone(),
                        metadata: Some(Json(metadata)),
                        cursor: format!(
                            "chat::{path}::{}::{}::{}",
                            event.checkpoint_id,
                            event.session_id,
                            start_index + message_index
                        ),
                    });
                }
            }
        }

        Ok(entries_by_path)
    }

    async fn list_chat_history_events(
        &self,
        path_candidates: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<CheckpointChatEvent>> {
        if path_candidates.is_empty() {
            return Ok(Vec::new());
        }

        let backend_config = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?;
        let repo_id = self.repo_identity.repo_id.as_str();

        if backend_config.events.has_clickhouse() {
            let sql = build_clickhouse_chat_history_sql(repo_id, path_candidates);
            let rows = self
                .query_clickhouse_data(&sql)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default();
            return rows
                .into_iter()
                .map(checkpoint_chat_event_from_row)
                .collect();
        }

        let sql = build_duckdb_chat_history_sql(repo_id, path_candidates);
        let duckdb_path = backend_config
            .events
            .resolve_duckdb_db_path_for_repo(&self.repo_root);
        let rows = self.query_duckdb_rows_at_path(&duckdb_path, &sql).await?;
        rows.into_iter()
            .map(normalise_duckdb_event_row)
            .map(checkpoint_chat_event_from_row)
            .collect()
    }
}

#[derive(Debug, Clone)]
struct CheckpointChatEvent {
    checkpoint_id: String,
    session_id: String,
    agent: String,
    event_time: DateTimeScalar,
    commit_sha: Option<String>,
    branch: Option<String>,
    strategy: Option<String>,
    files_touched: Vec<String>,
    payload: Option<Value>,
}

#[derive(Debug, Clone)]
struct SessionMessageRecord {
    role: ChatRole,
    raw_role: Option<String>,
    timestamp: DateTimeScalar,
    content: String,
}

fn build_project_clones_sql(
    repo_id: &str,
    branch: &str,
    project_path: &str,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let mut clauses = build_clone_filters(repo_id, filter);
    clauses.push(repo_path_prefix_clause("src.path", project_path));

    format!(
        "SELECT ce.source_artefact_id, ce.target_artefact_id, ce.relation_kind, ce.score, \
                ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM symbol_clone_edges ce \
           JOIN artefacts_current src ON src.repo_id = ce.repo_id AND src.branch = '{branch}' \
                                     AND src.symbol_id = ce.source_symbol_id \
           JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id AND tgt.branch = '{branch}' \
                                     AND tgt.symbol_id = ce.target_symbol_id \
          WHERE {} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        clauses.join(" AND "),
        branch = esc_pg(branch),
    )
}

fn build_artefact_clones_sql(
    repo_id: &str,
    branch: &str,
    artefact_id: &str,
    project_path: Option<&str>,
    filter: Option<&ClonesFilterInput>,
) -> String {
    let mut clauses = build_clone_filters(repo_id, filter);
    clauses.push(format!("ce.source_artefact_id = '{}'", esc_pg(artefact_id)));
    if let Some(project_path) = project_path {
        clauses.push(repo_path_prefix_clause("src.path", project_path));
    }

    format!(
        "SELECT ce.source_artefact_id, ce.target_artefact_id, ce.relation_kind, ce.score, \
                ce.semantic_score, ce.lexical_score, ce.structural_score, ce.explanation_json \
           FROM symbol_clone_edges ce \
           JOIN artefacts_current src ON src.repo_id = ce.repo_id AND src.branch = '{branch}' \
                                     AND src.symbol_id = ce.source_symbol_id \
           JOIN artefacts_current tgt ON tgt.repo_id = ce.repo_id AND tgt.branch = '{branch}' \
                                     AND tgt.symbol_id = ce.target_symbol_id \
          WHERE {} \
       ORDER BY ce.score DESC, tgt.path, COALESCE(tgt.symbol_fqn, ''), ce.target_artefact_id",
        clauses.join(" AND "),
        branch = esc_pg(branch),
    )
}

fn build_clone_filters(repo_id: &str, filter: Option<&ClonesFilterInput>) -> Vec<String> {
    let mut clauses = vec![format!("ce.repo_id = '{}'", esc_pg(repo_id))];
    if let Some(filter) = filter {
        if let Some(relation_kind) = filter.relation_kind() {
            clauses.push(format!("ce.relation_kind = '{}'", esc_pg(relation_kind)));
        }
        if let Some(min_score) = filter.min_score {
            clauses.push(format!("ce.score >= {}", min_score.clamp(0.0, 1.0)));
        }
    }

    clauses
}

fn build_clickhouse_chat_history_sql(
    repo_id: &str,
    path_candidates: &HashMap<String, Vec<String>>,
) -> String {
    let path_clause = path_candidates
        .values()
        .flat_map(|candidates| candidates.iter())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|candidate| format!("has(files_touched, '{}')", esc_ch(&candidate)))
        .collect::<Vec<_>>()
        .join(" OR ");

    format!(
        "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy, \
                files_touched, payload \
           FROM checkpoint_events \
          WHERE repo_id = '{repo_id}' \
            AND event_type = 'checkpoint_committed' \
            AND checkpoint_id != '' \
            AND session_id != '' \
            AND ({path_clause}) \
       ORDER BY event_time DESC, checkpoint_id DESC \
          LIMIT {limit} FORMAT JSON",
        repo_id = esc_ch(repo_id),
        limit = GRAPHQL_GIT_SCAN_LIMIT,
    )
}

fn build_duckdb_chat_history_sql(
    repo_id: &str,
    path_candidates: &HashMap<String, Vec<String>>,
) -> String {
    let path_clause = path_candidates
        .values()
        .flat_map(|candidates| candidates.iter())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|candidate| {
            format!(
                "files_touched LIKE '%\"{}\"%' ESCAPE '\\'",
                esc_pg(&escape_like_literal(&candidate))
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    format!(
        "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy, \
                files_touched, payload \
           FROM checkpoint_events \
          WHERE repo_id = '{repo_id}' \
            AND event_type = 'checkpoint_committed' \
            AND checkpoint_id <> '' \
            AND session_id <> '' \
            AND ({path_clause}) \
       ORDER BY event_time DESC, checkpoint_id DESC \
          LIMIT {limit}",
        repo_id = esc_pg(repo_id),
        limit = GRAPHQL_GIT_SCAN_LIMIT,
    )
}

fn clone_from_row(row: Value) -> Result<SemanticClone> {
    let source_artefact_id = required_string(&row, "source_artefact_id")?;
    let target_artefact_id = required_string(&row, "target_artefact_id")?;
    let relation_kind = required_string(&row, "relation_kind")?;

    let mut metadata = Map::new();
    if let Some(score) = optional_f64(&row, "semantic_score") {
        metadata.insert("semanticScore".to_string(), Value::from(score));
    }
    if let Some(score) = optional_f64(&row, "lexical_score") {
        metadata.insert("lexicalScore".to_string(), Value::from(score));
    }
    if let Some(score) = optional_f64(&row, "structural_score") {
        metadata.insert("structuralScore".to_string(), Value::from(score));
    }
    if let Some(explanation) = parse_json_column(row.get("explanation_json"))? {
        metadata.insert("explanation".to_string(), explanation);
    }

    Ok(SemanticClone {
        id: format!("clone::{source_artefact_id}::{target_artefact_id}::{relation_kind}").into(),
        source_artefact_id: source_artefact_id.into(),
        target_artefact_id: target_artefact_id.into(),
        relation_kind,
        score: required_f64(&row, "score")?,
        metadata: (!metadata.is_empty()).then_some(Json(Value::Object(metadata))),
        scope: ResolverScope::default(),
    })
}

fn checkpoint_chat_event_from_row(row: Value) -> Result<CheckpointChatEvent> {
    Ok(CheckpointChatEvent {
        checkpoint_id: required_string(&row, "checkpoint_id")?,
        session_id: required_string(&row, "session_id")?,
        agent: optional_string(&row, "agent").unwrap_or_else(|| "unknown".to_string()),
        event_time: parse_event_time(&required_string(&row, "event_time")?)?,
        commit_sha: optional_string(&row, "commit_sha"),
        branch: optional_string(&row, "branch"),
        strategy: optional_string(&row, "strategy"),
        files_touched: parse_string_array(row.get("files_touched"))?,
        payload: parse_payload(row.get("payload"))?,
    })
}

fn load_session_messages(
    repo_root: &Path,
    checkpoint_id: &str,
    session_id: &str,
    fallback_timestamp: &DateTimeScalar,
) -> Vec<SessionMessageRecord> {
    let Ok(content) = read_session_content_by_id(repo_root, checkpoint_id, session_id) else {
        return Vec::new();
    };

    parse_session_messages(&content, fallback_timestamp)
}

fn parse_session_messages(
    content: &SessionContentView,
    fallback_timestamp: &DateTimeScalar,
) -> Vec<SessionMessageRecord> {
    let session_timestamp = content
        .metadata
        .get("created_at")
        .and_then(parse_timestamp_value)
        .unwrap_or_else(|| fallback_timestamp.clone());

    let transcript_messages = extract_transcript_messages(&content.transcript)
        .into_iter()
        .filter_map(|message| {
            let text = extract_message_text(&message)?;
            let raw_role = extract_message_role(&message);
            Some(SessionMessageRecord {
                role: ChatRole::from_raw(raw_role.as_deref()),
                raw_role,
                timestamp: extract_message_timestamp(&message)
                    .unwrap_or_else(|| session_timestamp.clone()),
                content: text,
            })
        })
        .collect::<Vec<_>>();

    if !transcript_messages.is_empty() {
        return transcript_messages;
    }

    split_prompts(&content.prompts)
        .into_iter()
        .map(|prompt| SessionMessageRecord {
            role: ChatRole::User,
            raw_role: Some("user".to_string()),
            timestamp: session_timestamp.clone(),
            content: prompt,
        })
        .collect()
}

fn extract_transcript_messages(transcript: &str) -> Vec<Value> {
    let trimmed = transcript.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        let extracted = collect_message_values(&value);
        if !extracted.is_empty() {
            return extracted;
        }
    }

    transcript
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(trimmed).ok()
        })
        .flat_map(|value| collect_message_values(&value))
        .collect()
}

fn collect_message_values(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values.to_vec(),
        Value::Object(map) => match map.get("messages").and_then(Value::as_array) {
            Some(messages) => messages.to_vec(),
            None if map.contains_key("role")
                || map.contains_key("type")
                || map.contains_key("message") =>
            {
                vec![value.clone()]
            }
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_message_role(value: &Value) -> Option<String> {
    value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/message/role").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(flatten_text_value)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(map) => map
            .get("text")
            .and_then(flatten_text_value)
            .or_else(|| map.get("content").and_then(flatten_text_value))
            .or_else(|| map.get("input").and_then(flatten_text_value)),
        _ => None,
    }
}

fn extract_message_timestamp(value: &Value) -> Option<DateTimeScalar> {
    value
        .get("timestamp")
        .and_then(parse_timestamp_value)
        .or_else(|| {
            value
                .pointer("/time/completed")
                .and_then(parse_timestamp_value)
        })
        .or_else(|| {
            value
                .pointer("/time/created")
                .and_then(parse_timestamp_value)
        })
        .or_else(|| {
            value
                .pointer("/message/timestamp")
                .and_then(parse_timestamp_value)
        })
        .or_else(|| value.get("created_at").and_then(parse_timestamp_value))
}

fn parse_timestamp_value(value: &Value) -> Option<DateTimeScalar> {
    match value {
        Value::String(raw) => parse_event_time(raw).ok(),
        Value::Number(number) => number.as_i64().and_then(unix_timestamp_to_scalar),
        _ => None,
    }
}

fn unix_timestamp_to_scalar(seconds: i64) -> Option<DateTimeScalar> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .and_then(|timestamp| DateTimeScalar::from_rfc3339(timestamp.to_rfc3339()).ok())
}

fn split_prompts(prompts: &str) -> Vec<String> {
    prompts
        .split("\n\n---\n\n")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn build_chat_metadata(
    event: &CheckpointChatEvent,
    raw_role: Option<&str>,
    message_index: usize,
) -> Value {
    let mut metadata = Map::new();
    metadata.insert(
        "checkpointId".to_string(),
        Value::String(event.checkpoint_id.clone()),
    );
    metadata.insert(
        "messageIndex".to_string(),
        Value::from(message_index as i64),
    );
    if let Some(raw_role) = raw_role
        && !raw_role.trim().is_empty()
    {
        metadata.insert(
            "rawRole".to_string(),
            Value::String(raw_role.trim().to_string()),
        );
    }
    if let Some(commit_sha) = event.commit_sha.as_ref() {
        metadata.insert("commitSha".to_string(), Value::String(commit_sha.clone()));
    }
    if let Some(branch) = event.branch.as_ref() {
        metadata.insert("branch".to_string(), Value::String(branch.clone()));
    }
    if let Some(strategy) = event.strategy.as_ref() {
        metadata.insert("strategy".to_string(), Value::String(strategy.clone()));
    }
    if let Some(payload) = event.payload.as_ref() {
        metadata.insert("eventPayload".to_string(), payload.clone());
    }

    Value::Object(metadata)
}

fn dedup_paths(paths: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for path in paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            unique.push(trimmed.to_string());
        }
    }

    unique
}

fn build_path_candidates(path: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        candidates.push(raw.to_string());
    }

    let normalised = normalise_repo_path(raw);
    if !normalised.is_empty() {
        candidates.push(normalised.clone());
        candidates.push(format!("./{normalised}"));
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn files_touched_matches(files_touched: &[String], candidates: &[String]) -> bool {
    let candidate_set = candidates
        .iter()
        .map(|candidate| normalise_repo_path(candidate))
        .collect::<HashSet<_>>();

    files_touched
        .iter()
        .map(|path| normalise_repo_path(path))
        .any(|path| candidate_set.contains(&path))
}

fn normalise_repo_path(path: &str) -> String {
    let mut normalised = path.trim().replace('\\', "/");
    while normalised.starts_with("./") {
        normalised = normalised[2..].to_string();
    }
    normalised.trim_start_matches('/').to_string()
}

fn repo_path_prefix_clause(column: &str, project_path: &str) -> String {
    let prefix = format!("{}/%", escape_like_pattern(project_path));
    format!(
        "({column} = '{path}' OR {like_clause})",
        column = column,
        path = esc_pg(project_path),
        like_clause = sql_like_with_escape(column, &prefix),
    )
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_f64(row: &Value, key: &str) -> Result<f64> {
    optional_f64(row, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

fn optional_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64).or_else(|| {
        row.get(key)
            .and_then(Value::as_i64)
            .map(|value| value as f64)
    })
}

fn parse_json_column(value: Option<&Value>) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }

            serde_json::from_str(trimmed)
                .map(Some)
                .with_context(|| "parsing JSON payload column")
        }
        Some(other) => Ok(Some(other.clone())),
    }
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

fn parse_string_array(value: Option<&Value>) -> Result<Vec<String>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }
            let parsed: Value =
                serde_json::from_str(trimmed).context("parsing `files_touched` JSON")?;
            parse_string_array(Some(&parsed))
        }
        Some(other) => bail!("unexpected `files_touched` value in events row: {other}"),
    }
}

fn parse_payload(value: Option<&Value>) -> Result<Option<Value>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed = serde_json::from_str(trimmed)
                .unwrap_or_else(|_| Value::String(trimmed.to_string()));
            Ok(Some(parsed))
        }
        Some(other) => Ok(Some(other.clone())),
    }
}

fn parse_event_time(raw: &str) -> Result<DateTimeScalar> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("event time is empty");
    }

    if let Ok(value) = DateTimeScalar::from_rfc3339(trimmed.to_string()) {
        return Ok(value);
    }

    for pattern in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, pattern) {
            let timestamp = Utc.from_utc_datetime(&naive).to_rfc3339();
            return DateTimeScalar::from_rfc3339(timestamp)
                .context("formatting event timestamp as RFC 3339");
        }
    }

    if let Ok(unix_seconds) = trimmed.parse::<i64>()
        && let Some(timestamp) = Utc.timestamp_opt(unix_seconds, 0).single()
    {
        return DateTimeScalar::from_rfc3339(timestamp.to_rfc3339())
            .context("formatting unix event timestamp as RFC 3339");
    }

    bail!("unsupported event timestamp `{trimmed}`")
}

fn escape_like_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
