use super::row::parse_event_time;
use super::types::SessionMessageRecord;
use crate::graphql::types::{ChatRole, DateTimeScalar};
use crate::host::checkpoints::strategy::manual_commit::{
    SessionContentView, read_session_content_by_id,
};
use chrono::{TimeZone, Utc};
use serde_json::Value;
use std::path::Path;

pub(super) fn load_session_messages(
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
