use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};

pub(super) fn blocking_query_rows(
    endpoint: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<Vec<Value>> {
    let mut query = sql.trim().to_string();
    if !query.to_ascii_uppercase().contains("FORMAT JSON") {
        query.push_str(" FORMAT JSON");
    }
    let raw = blocking_exec(endpoint, user, password, &query)?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing ClickHouse JSON: {raw}"))?;
    Ok(parsed
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

pub(super) fn blocking_exec(
    endpoint: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<String> {
    let client = blocking_client()?;
    let mut request = client.post(endpoint).body(sql.to_string());
    if let Some(username) = user {
        request = request.basic_auth(username, Some(password.unwrap_or("")));
    }
    let response = request
        .send()
        .context("sending interaction request to ClickHouse")?;
    let status = response.status();
    let body = response
        .text()
        .context("reading ClickHouse response body")?;
    if !status.is_success() {
        let detail = body.trim();
        if detail.is_empty() {
            bail!("ClickHouse request failed with status {status}");
        }
        bail!("ClickHouse request failed with status {status}: {detail}");
    }
    Ok(body)
}

fn blocking_client() -> Result<&'static reqwest::blocking::Client> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    let result = CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| format!("{err:#}"))
    });
    match result {
        Ok(client) => Ok(client),
        Err(err) => Err(anyhow!("building blocking ClickHouse client: {err}")),
    }
}

pub(super) fn session_from_row(row: &Value) -> Result<InteractionSession> {
    Ok(InteractionSession {
        session_id: required_string(row, "session_id")?,
        repo_id: required_string(row, "repo_id")?,
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        first_prompt: optional_string(row, "first_prompt"),
        transcript_path: optional_string(row, "transcript_path"),
        worktree_path: optional_string(row, "worktree_path"),
        worktree_id: optional_string(row, "worktree_id"),
        started_at: optional_string(row, "started_at"),
        ended_at: empty_to_none(optional_string(row, "ended_at")),
        last_event_at: optional_string(row, "last_event_at"),
        updated_at: optional_string(row, "updated_at"),
    })
}

pub(super) fn turn_from_row(row: &Value) -> Result<InteractionTurn> {
    let files_modified = match row.get("files_modified") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(other) => bail!("unexpected files_modified payload: {other}"),
        None => Vec::new(),
    };
    let has_token_usage = row
        .get("has_token_usage")
        .and_then(Value::as_u64)
        .unwrap_or_default()
        == 1;
    Ok(InteractionTurn {
        turn_id: required_string(row, "turn_id")?,
        session_id: required_string(row, "session_id")?,
        repo_id: required_string(row, "repo_id")?,
        turn_number: row
            .get("turn_number")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        prompt: optional_string(row, "prompt"),
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        started_at: optional_string(row, "started_at"),
        ended_at: empty_to_none(optional_string(row, "ended_at")),
        token_usage: has_token_usage.then(|| TokenUsageMetadata {
            input_tokens: row
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cache_creation_tokens: row
                .get("cache_creation_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            cache_read_tokens: row
                .get("cache_read_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            output_tokens: row
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            api_call_count: row
                .get("api_call_count")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            subagent_tokens: None,
        }),
        files_modified,
        checkpoint_id: empty_to_none(optional_string(row, "checkpoint_id")),
        updated_at: optional_string(row, "updated_at"),
    })
}

pub(super) fn event_from_row(row: &Value) -> Result<InteractionEvent> {
    let event_type_raw = required_string(row, "event_type")?;
    let payload = row
        .get("payload")
        .and_then(Value::as_str)
        .map(serde_json::from_str::<Value>)
        .transpose()
        .context("parsing interaction event payload")?
        .unwrap_or_else(|| Value::Object(Default::default()));
    Ok(InteractionEvent {
        event_id: required_string(row, "event_id")?,
        session_id: required_string(row, "session_id")?,
        turn_id: empty_to_none(optional_string(row, "turn_id")),
        repo_id: required_string(row, "repo_id")?,
        event_type: InteractionEventType::parse(&event_type_raw)
            .ok_or_else(|| anyhow!("unknown interaction event type `{event_type_raw}`"))?,
        event_time: required_string(row, "event_time")?,
        agent_type: optional_string(row, "agent_type"),
        model: optional_string(row, "model"),
        payload,
    })
}

fn required_string(row: &Value, field: &str) -> Result<String> {
    row.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("missing `{field}` in ClickHouse interaction row"))
}

fn optional_string(row: &Value, field: &str) -> String {
    row.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn empty_to_none(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
