use super::types::CheckpointChatEvent;
use crate::graphql::types::DateTimeScalar;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::Value;
use std::convert::TryFrom;

#[allow(dead_code)]
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

#[allow(dead_code)]
pub(super) fn parse_event_time(raw: &str) -> Result<DateTimeScalar> {
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

#[allow(dead_code)]
fn required_f64(row: &Value, key: &str) -> Result<f64> {
    optional_f64(row, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

#[allow(dead_code)]
fn optional_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64).or_else(|| {
        row.get(key)
            .and_then(Value::as_i64)
            .map(|value| value as f64)
    })
}

#[allow(dead_code)]
fn optional_i32(row: &Value, key: &str) -> Option<i32> {
    row.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
pub(super) fn escape_like_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
